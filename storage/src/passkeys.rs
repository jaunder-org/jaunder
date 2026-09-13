//! Durable Passkey records and short-lived `WebAuthn` ceremony state.
//!
//! Raw ceremony handles never cross the SQL boundary. Their SHA-256 digest is
//! the sole lookup key, so a database disclosure cannot replay a live ceremony.

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use sha2::{Digest as _, Sha256};
use sqlx::{Database, Decode, Encode, Executor, Pool, Type};
use thiserror::Error;

use crate::{WriteTransaction, backend::Backend, sql::QueryStorageExt};
use common::{ids::UserId, time::UtcInstant, token::TokenHash};
use host::passkey::{AuthenticationState, Credential, RegistrationState, UserHandle};
use macros::StrNewtype;

const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";

fn lowercase_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(char::from(LOWER_HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(LOWER_HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

/// A non-blank, presentation-only name chosen for a Passkey.
#[derive(Clone, Debug, Eq, PartialEq, StrNewtype)]
pub struct PasskeyLabel(String);

/// Error returned when a Passkey label is blank.
#[derive(Debug, Error)]
#[error("passkey label must not be blank")]
pub struct InvalidPasskeyLabel;

impl FromStr for PasskeyLabel {
    type Err = InvalidPasskeyLabel;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err(InvalidPasskeyLabel);
        }
        Ok(Self(value.to_owned()))
    }
}

/// A globally unique, opaque `WebAuthn` credential identifier.
#[derive(Clone, PartialEq, Eq, Hash, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct PasskeyCredentialId(String);

impl PasskeyCredentialId {
    #[must_use]
    pub fn from_credential(credential: &Credential) -> Self {
        Self::from_bytes(credential.credential_id())
    }

    /// Encodes the opaque credential bytes at the trusted adapter boundary.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }
}

impl FromStr for PasskeyCredentialId {
    type Err = base64::DecodeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        URL_SAFE_NO_PAD.decode(value)?;
        Ok(Self(value.to_owned()))
    }
}

/// The stable, random 16-byte handle `WebAuthn` uses to identify a User.
#[derive(Clone, PartialEq, Eq, Hash, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct PasskeyUserHandle(String);

/// Error returned for an invalid stored User handle.
#[derive(Debug, Error)]
#[error("passkey user handle must be exactly 16 bytes encoded as lowercase hexadecimal")]
pub struct InvalidPasskeyUserHandle;

impl PasskeyUserHandle {
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 16];
        rand::rng().fill_bytes(&mut bytes);
        Self(lowercase_hex(&bytes))
    }

    /// Returns the relying-party adapter value without exposing its bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if a malformed value somehow bypassed this type's
    /// validated constructor before persistence.
    pub fn adapter_handle(&self) -> Result<UserHandle, InvalidPasskeyUserHandle> {
        fn nibble(byte: u8) -> Option<u8> {
            match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            }
        }

        let mut bytes = [0_u8; 16];
        for (index, pair) in self.0.as_bytes().chunks_exact(2).enumerate() {
            let high = nibble(pair[0]).ok_or(InvalidPasskeyUserHandle)?;
            let low = nibble(pair[1]).ok_or(InvalidPasskeyUserHandle)?;
            bytes[index] = (high << 4) | low;
        }
        Ok(UserHandle::new(bytes))
    }

    /// Encodes the adapter's opaque bytes for durable lookup.
    #[must_use]
    pub fn from_adapter_handle(handle: &UserHandle) -> Self {
        Self(lowercase_hex(handle.as_bytes()))
    }
}

impl FromStr for PasskeyUserHandle {
    type Err = InvalidPasskeyUserHandle;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 32
            || !value.bytes().all(|byte| {
                byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
            })
        {
            return Err(InvalidPasskeyUserHandle);
        }
        Ok(Self(value.to_owned()))
    }
}

/// A browser-presented, high-entropy ceremony handle. It is deliberately not SQL-bindable.
pub struct RawPasskeyCeremonyHandle(String);

impl RawPasskeyCeremonyHandle {
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Returns the handle for the browser ceremony response boundary.
    ///
    /// This is the sole outbound transport representation. It MUST NOT enter
    /// logs, telemetry, user-visible errors, or SQL; call [`Self::hash`] before
    /// every storage lookup.
    #[must_use]
    pub fn expose_to_browser(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn hash(&self) -> StoredPasskeyCeremonyHandleHash {
        let digest = Sha256::digest(self.0.as_bytes());
        StoredPasskeyCeremonyHandleHash(URL_SAFE_NO_PAD.encode(digest))
    }
}

impl fmt::Debug for RawPasskeyCeremonyHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RawPasskeyCeremonyHandle([REDACTED])")
    }
}

/// Error returned when a browser-provided ceremony handle is malformed.
#[derive(Debug, Error)]
pub enum InvalidRawPasskeyCeremonyHandle {
    #[error("ceremony handle is not canonical base64url without padding")]
    Encoding(#[from] base64::DecodeError),
    #[error("ceremony handle must contain exactly 32 bytes")]
    Length,
    #[error("ceremony handle must use canonical base64url without padding")]
    NonCanonical,
}

impl FromStr for RawPasskeyCeremonyHandle {
    type Err = InvalidRawPasskeyCeremonyHandle;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let bytes = URL_SAFE_NO_PAD.decode(value)?;
        if bytes.len() != 32 {
            return Err(InvalidRawPasskeyCeremonyHandle::Length);
        }
        if URL_SAFE_NO_PAD.encode(&bytes) != value {
            return Err(InvalidRawPasskeyCeremonyHandle::NonCanonical);
        }
        Ok(Self(value.to_owned()))
    }
}

/// The SHA-256 digest of a raw ceremony handle, persisted as the lookup key.
#[derive(Clone, Debug, Eq, PartialEq, Hash, StrNewtype)]
pub struct StoredPasskeyCeremonyHandleHash(String);

/// Error returned for an invalid persisted ceremony-handle hash.
#[derive(Debug, Error)]
pub enum InvalidStoredPasskeyCeremonyHandleHash {
    #[error("ceremony-handle hash is not base64url")]
    Encoding(#[from] base64::DecodeError),
    #[error("ceremony-handle hash must contain a SHA-256 digest")]
    Length,
}

impl FromStr for StoredPasskeyCeremonyHandleHash {
    type Err = InvalidStoredPasskeyCeremonyHandleHash;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if URL_SAFE_NO_PAD.decode(value)?.len() != 32 {
            return Err(InvalidStoredPasskeyCeremonyHandleHash::Length);
        }
        Ok(Self(value.to_owned()))
    }
}

/// One durable Passkey owned by a User.
#[derive(Clone, Debug)]
pub struct PasskeyCredential {
    pub id: PasskeyCredentialId,
    pub user_id: UserId,
    pub label: PasskeyLabel,
    pub credential: Credential,
    pub created_at: UtcInstant,
    pub last_used_at: Option<UtcInstant>,
}

/// A registration ceremony claimed exactly once by its handle hash.
#[derive(Clone, Debug)]
pub struct RegistrationCeremony {
    pub user_id: UserId,
    pub session_token_hash: TokenHash,
    pub label: PasskeyLabel,
    pub origin: String,
    pub rp_id: String,
    pub state: RegistrationState,
}

/// An authentication ceremony claimed exactly once by its handle hash.
#[derive(Clone, Debug)]
pub struct AuthenticationCeremony {
    pub origin: String,
    pub rp_id: String,
    pub state: AuthenticationState,
}

/// Errors produced by malformed stored Passkey serialization.
#[derive(Debug, Error)]
pub enum PasskeyDecodeError {
    #[error("stored passkey credential identity disagrees with its credential payload")]
    CredentialIdentityMismatch,
    #[error("stored passkey ceremony purpose disagrees with its state type")]
    CeremonyPurposeMismatch,
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

fn decode_error(error: impl std::error::Error + Send + Sync + 'static) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(error))
}

fn decode_credential(
    id: PasskeyCredentialId,
    user_id: UserId,
    label: PasskeyLabel,
    encoded: &str,
    created_at: UtcInstant,
    last_used_at: Option<UtcInstant>,
) -> Result<PasskeyCredential, sqlx::Error> {
    let credential = serde_json::from_str::<Credential>(encoded).map_err(decode_error)?;
    if PasskeyCredentialId::from_credential(&credential) != id {
        return Err(decode_error(PasskeyDecodeError::CredentialIdentityMismatch));
    }
    Ok(PasskeyCredential {
        id,
        user_id,
        label,
        credential,
        created_at,
        last_used_at,
    })
}

/// Object-safe persistence boundary for durable Passkeys and transient ceremonies.
#[async_trait]
pub trait PasskeyStorage: Send + Sync {
    async fn user_handle(&self, user_id: UserId) -> sqlx::Result<Option<PasskeyUserHandle>>;
    async fn user_for_handle(&self, handle: &PasskeyUserHandle) -> sqlx::Result<Option<UserId>>;
    async fn insert_credential(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        label: &PasskeyLabel,
        credential: &Credential,
    ) -> sqlx::Result<()>;
    async fn list_credentials(&self, user_id: UserId) -> sqlx::Result<Vec<PasskeyCredential>>;
    async fn credential_for_user(
        &self,
        user_id: UserId,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<Option<PasskeyCredential>>;
    async fn delete_credential(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<bool>;

    /// Reads a credential by its globally unique identity for signed-out discovery.
    async fn credential(
        &self,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<Option<PasskeyCredential>>;
    async fn create_registration_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        ceremony: &RegistrationCeremony,
        expires_at: UtcInstant,
    ) -> sqlx::Result<()>;
    async fn claim_registration_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        now: UtcInstant,
    ) -> sqlx::Result<Option<RegistrationCeremony>>;
    async fn create_authentication_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        ceremony: &AuthenticationCeremony,
        expires_at: UtcInstant,
    ) -> sqlx::Result<()>;
    async fn claim_authentication_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        now: UtcInstant,
    ) -> sqlx::Result<Option<AuthenticationCeremony>>;
    async fn cleanup_ceremonies(
        &self,
        transaction: &mut WriteTransaction,
        now: UtcInstant,
    ) -> sqlx::Result<()>;
}

/// Generic, shared-SQL implementation of [`PasskeyStorage`].
pub struct PasskeyStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> PasskeyStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> PasskeyStorage for PasskeyStore<DB>
where
    DB: Backend,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> PasskeyUserHandle: Encode<'q, DB> + Type<DB>,
    for<'q> PasskeyCredentialId: Encode<'q, DB> + Type<DB>,
    for<'q> PasskeyLabel: Encode<'q, DB> + Type<DB>,
    for<'q> StoredPasskeyCeremonyHandleHash: Encode<'q, DB> + Type<DB>,
    for<'q> TokenHash: Encode<'q, DB> + Type<DB>,
    for<'q> UserId: Encode<'q, DB> + Type<DB>,
    for<'q> StoredPasskeySerialization: Encode<'q, DB> + Type<DB>,
    for<'q> StoredPasskeyText: Encode<'q, DB> + Type<DB>,
    for<'r> PasskeyUserHandle: Decode<'r, DB> + Type<DB>,
    for<'r> PasskeyCredentialId: Decode<'r, DB> + Type<DB>,
    for<'r> PasskeyLabel: Decode<'r, DB> + Type<DB>,
    for<'r> TokenHash: Decode<'r, DB> + Type<DB>,
    for<'r> UserId: Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: Decode<'r, DB> + Type<DB>,
    for<'r> StoredPasskeySerialization: Decode<'r, DB> + Type<DB>,
    for<'r> StoredPasskeyText: Decode<'r, DB> + Type<DB>,
    for<'r> StoredPasskeyPurpose: Decode<'r, DB> + Type<DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'r> String: Decode<'r, DB> + Type<DB>,
{
    async fn user_handle(&self, user_id: UserId) -> sqlx::Result<Option<PasskeyUserHandle>> {
        sqlx::query_scalar("SELECT user_handle FROM passkey_user_handles WHERE user_id = $1")
            .bind_storage(user_id)
            .fetch_optional(&self.pool)
            .await
    }

    async fn user_for_handle(&self, handle: &PasskeyUserHandle) -> sqlx::Result<Option<UserId>> {
        sqlx::query_scalar("SELECT user_id FROM passkey_user_handles WHERE user_handle = $1")
            .bind_storage(handle)
            .fetch_optional(&self.pool)
            .await
    }

    async fn insert_credential(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        label: &PasskeyLabel,
        credential: &Credential,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        let id = PasskeyCredentialId::from_credential(credential);
        let encoded = serde_json::to_string(credential).map_err(decode_error)?;
        sqlx::query("INSERT INTO passkey_credentials (credential_id, user_id, label, credential, created_at) VALUES ($1, $2, $3, $4, $5)")
            .bind_storage(id)
            .bind_storage(user_id)
            .bind_storage(label)
            .bind_storage(StoredPasskeySerialization(encoded))
            .bind_storage(UtcInstant::now())
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn list_credentials(&self, user_id: UserId) -> sqlx::Result<Vec<PasskeyCredential>> {
        let rows = sqlx::query_as::<_, (PasskeyCredentialId, UserId, PasskeyLabel, StoredPasskeySerialization, UtcInstant, Option<UtcInstant>)>(
            "SELECT credential_id, user_id, label, credential, created_at, last_used_at FROM passkey_credentials WHERE user_id = $1 ORDER BY created_at, credential_id",
        ).bind_storage(user_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|(id, user_id, label, encoded, created_at, last_used_at)| {
                decode_credential(id, user_id, label, &encoded.0, created_at, last_used_at)
            })
            .collect()
    }

    async fn credential_for_user(
        &self,
        user_id: UserId,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<Option<PasskeyCredential>> {
        let row = sqlx::query_as::<_, (PasskeyCredentialId, UserId, PasskeyLabel, StoredPasskeySerialization, UtcInstant, Option<UtcInstant>)>(
            "SELECT credential_id, user_id, label, credential, created_at, last_used_at FROM passkey_credentials WHERE user_id = $1 AND credential_id = $2",
        ).bind_storage(user_id).bind_storage(credential_id).fetch_optional(&self.pool).await?;
        row.map(|(id, user_id, label, encoded, created_at, last_used_at)| {
            decode_credential(id, user_id, label, &encoded.0, created_at, last_used_at)
        })
        .transpose()
    }

    async fn delete_credential(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<bool> {
        let connection = DB::write_connection(transaction)?;
        let deleted = sqlx::query_scalar::<_, PasskeyCredentialId>(
            "DELETE FROM passkey_credentials WHERE user_id = $1 AND credential_id = $2 \
             RETURNING credential_id",
        )
        .bind_storage(user_id)
        .bind_storage(credential_id)
        .fetch_optional(&mut *connection)
        .await?;
        Ok(deleted.is_some())
    }

    async fn credential(
        &self,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<Option<PasskeyCredential>> {
        let row = sqlx::query_as::<
            _,
            (
                PasskeyCredentialId,
                UserId,
                PasskeyLabel,
                StoredPasskeySerialization,
                UtcInstant,
                Option<UtcInstant>,
            ),
        >(
            "SELECT credential_id, user_id, label, credential, created_at, last_used_at
             FROM passkey_credentials WHERE credential_id = $1",
        )
        .bind_storage(credential_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(id, user_id, label, encoded, created_at, last_used_at)| {
            decode_credential(id, user_id, label, &encoded.0, created_at, last_used_at)
        })
        .transpose()
    }

    async fn create_registration_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        ceremony: &RegistrationCeremony,
        expires_at: UtcInstant,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("INSERT INTO passkey_registration_ceremonies (handle_hash, purpose, user_id, session_token_hash, label, origin, rp_id, state, expires_at) VALUES ($1, 'registration', $2, $3, $4, $5, $6, $7, $8)")
            .bind_storage(handle_hash)
            .bind_storage(ceremony.user_id)
            .bind_storage(&ceremony.session_token_hash)
            .bind_storage(&ceremony.label)
            .bind_storage(StoredPasskeyText(ceremony.origin.clone()))
            .bind_storage(StoredPasskeyText(ceremony.rp_id.clone()))
            .bind_storage(StoredPasskeySerialization(
                serde_json::to_string(&ceremony.state).map_err(decode_error)?,
            ))
            .bind_storage(expires_at)
            .execute(&mut *connection).await?;
        Ok(())
    }

    async fn claim_registration_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        now: UtcInstant,
    ) -> sqlx::Result<Option<RegistrationCeremony>> {
        let connection = DB::write_connection(transaction)?;
        let row = sqlx::query_as::<_, (StoredPasskeyPurpose, UserId, TokenHash, PasskeyLabel, StoredPasskeyText, StoredPasskeyText, StoredPasskeySerialization)>(
            "UPDATE passkey_registration_ceremonies SET claimed_at = $1 WHERE handle_hash = $2 AND purpose = 'registration' AND claimed_at IS NULL AND expires_at > $1 RETURNING purpose, user_id, session_token_hash, label, origin, rp_id, state",
        ).bind_storage(now).bind_storage(handle_hash).fetch_optional(&mut *connection).await?;
        row.map(
            |(purpose, user_id, session_token_hash, label, origin, rp_id, state)| {
                if purpose.0 != "registration" {
                    return Err(decode_error(PasskeyDecodeError::CeremonyPurposeMismatch));
                }
                Ok(RegistrationCeremony {
                    user_id,
                    session_token_hash,
                    label,
                    origin: origin.0,
                    rp_id: rp_id.0,
                    state: serde_json::from_str(&state.0).map_err(decode_error)?,
                })
            },
        )
        .transpose()
    }

    async fn create_authentication_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        ceremony: &AuthenticationCeremony,
        expires_at: UtcInstant,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("INSERT INTO passkey_authentication_ceremonies (handle_hash, purpose, origin, rp_id, state, expires_at) VALUES ($1, 'authentication', $2, $3, $4, $5)")
            .bind_storage(handle_hash).bind_storage(StoredPasskeyText(ceremony.origin.clone())).bind_storage(StoredPasskeyText(ceremony.rp_id.clone()))
            .bind_storage(StoredPasskeySerialization(serde_json::to_string(&ceremony.state).map_err(decode_error)?)).bind_storage(expires_at)
            .execute(&mut *connection).await?;
        Ok(())
    }

    async fn claim_authentication_ceremony(
        &self,
        transaction: &mut WriteTransaction,
        handle_hash: &StoredPasskeyCeremonyHandleHash,
        now: UtcInstant,
    ) -> sqlx::Result<Option<AuthenticationCeremony>> {
        let connection = DB::write_connection(transaction)?;
        let row = sqlx::query_as::<_, (StoredPasskeyPurpose, StoredPasskeyText, StoredPasskeyText, StoredPasskeySerialization)>(
            "UPDATE passkey_authentication_ceremonies SET claimed_at = $1 WHERE handle_hash = $2 AND purpose = 'authentication' AND claimed_at IS NULL AND expires_at > $1 RETURNING purpose, origin, rp_id, state",
        ).bind_storage(now).bind_storage(handle_hash).fetch_optional(&mut *connection).await?;
        row.map(|(purpose, origin, rp_id, state)| {
            if purpose.0 != "authentication" {
                return Err(decode_error(PasskeyDecodeError::CeremonyPurposeMismatch));
            }
            Ok(AuthenticationCeremony {
                origin: origin.0,
                rp_id: rp_id.0,
                state: serde_json::from_str(&state.0).map_err(decode_error)?,
            })
        })
        .transpose()
    }

    async fn cleanup_ceremonies(
        &self,
        transaction: &mut WriteTransaction,
        now: UtcInstant,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM passkey_registration_ceremonies WHERE claimed_at IS NOT NULL OR expires_at <= $1").bind_storage(now).execute(&mut *connection).await?;
        sqlx::query("DELETE FROM passkey_authentication_ceremonies WHERE claimed_at IS NOT NULL OR expires_at <= $1").bind_storage(now).execute(&mut *connection).await?;
        Ok(())
    }
}

/// Trusted JSON copied from the adapter; only this private role is SQL-bindable.

#[derive(macros::SqlxBridge)]
pub(crate) struct StoredPasskeySerialization(String);

/// Trusted exact origin/RP strings copied from configuration; only this private role is SQL-bindable.
#[derive(macros::SqlxBridge)]
pub(crate) struct StoredPasskeyText(String);

/// Database-only ceremony-purpose discriminator; it is decoded defensively
/// rather than trusted as an untyped string.
#[derive(macros::SqlxBridge)]
struct StoredPasskeyPurpose(String);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, SeedUser, backends};
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    const CREDENTIAL_FIXTURE: &str = r#"{"cred_id":"uZcVDBVS68E_MtAgeQpElJxldF_6cY9sSvbWqx_qRh8wiu42lyRBRmh5yFeD_r9k130dMbFHBHI9RTFgdJQIzQ","cred":{"type_":"ES256","key":{"EC_EC2":{"curve":"SECP256R1","x":[194,126,127,109,252,23,131,21,252,6,223,99,44,254,140,27,230,17,94,5,133,28,104,41,144,69,171,149,161,26,200,243],"y":[143,123,183,156,24,178,21,248,117,159,162,69,171,52,188,252,26,59,6,47,103,92,19,58,117,103,249,0,219,8,95,196]}}},"counter":2,"user_verified":false,"backup_eligible":false,"backup_state":false,"registration_policy":"preferred","extensions":{"cred_protect":"NotRequested","hmac_create_secret":"NotRequested"},"attestation":{"data":{"Basic":["MIICvTCCAaWgAwIBAgIEK_F8eDANBgkqhkiG9w0BAQsFADAuMSwwKgYDVQQDEyNZdWJpY28gVTJGIFJvb3QgQ0EgU2VyaWFsIDQ1NzIwMDYzMTAgFw0xNDA4MDEwMDAwMDBaGA8yMDUwMDkwNDAwMDAwMFowbjELMAkGA1UEBhMCU0UxEjAQBgNVBAoMCVl1YmljbyBBQjEiMCAGA1UECwwZQXV0aGVudGljYXRvciBBdHRlc3RhdGlvbjEnMCUGA1UEAwweWXViaWNvIFUyRiBFRSBTZXJpYWwgNzM3MjQ2MzI4MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEdMLHhCPIcS6bSPJZWGb8cECuTN8H13fVha8Ek5nt-pI8vrSflxb59Vp4bDQlH8jzXj3oW1ZwUDjHC6EnGWB5i6NsMGowIgYJKwYBBAGCxAoCBBUxLjMuNi4xLjQuMS40MTQ4Mi4xLjcwEwYLKwYBBAGC5RwCAQEEBAMCAiQwIQYLKwYBBAGC5RwBAQQEEgQQxe9V_62aS5-1gK3rr-Am0DAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQCLbpN2nXhNbunZANJxAn_Cd-S4JuZsObnUiLnLLS0FPWa01TY8F7oJ8bE-aFa4kTe6NQQfi8-yiZrQ8N-JL4f7gNdQPSrH-r3iFd4SvroDe1jaJO4J9LeiFjmRdcVa-5cqNF4G1fPCofvw9W4lKnObuPakr0x_icdVq1MXhYdUtQk6Zr5mBnc4FhN9qi7DXqLHD5G7ZFUmGwfIcD2-0m1f1mwQS8yRD5-_aDCf3vutwddoi3crtivzyromwbKklR4qHunJ75LGZLZA8pJ_mXnUQ6TTsgRqPvPXgQPbSyGMf2z_DIPbQqCD_Bmc4dj9o6LozheBdDtcZCAjSPTAd_ui"]},"metadata":"None"},"attestation_format":"Packed"}"#;

    fn credential_fixture() -> Credential {
        Credential::deserialize_for_test(&format!(r#"{{"cred":{CREDENTIAL_FIXTURE}}}"#))
            .expect("pinned fork fixture decodes")
    }

    async fn seed_user(env: &crate::test_support::TestEnv) -> UserId {
        SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id
    }

    async fn insert_credential(
        env: &crate::test_support::TestEnv,
        user_id: UserId,
        label: &str,
    ) -> Credential {
        let credential = credential_fixture();
        let stored_credential = credential.clone();
        let label = label.parse().unwrap();
        let passkeys = env.passkeys();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .insert_credential(transaction, user_id, &label, &stored_credential)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        credential
    }
    fn credential_fixture_with_id(id: &str) -> Credential {
        let fixture = CREDENTIAL_FIXTURE.replacen(
            "uZcVDBVS68E_MtAgeQpElJxldF_6cY9sSvbWqx_qRh8wiu42lyRBRmh5yFeD_r9k130dMbFHBHI9RTFgdJQIzQ",
            id,
            1,
        );
        Credential::deserialize_for_test(&format!(r#"{{"cred":{fixture}}}"#))
            .expect("fixture with replacement credential identifier decodes")
    }

    #[test]
    fn opaque_identifiers_round_trip_at_adapter_boundaries() {
        let handle = PasskeyUserHandle::generate();
        assert_eq!(format!("{handle:?}"), "PasskeyUserHandle([redacted])");
        let adapter_handle = handle.adapter_handle().unwrap();
        assert_eq!(
            PasskeyUserHandle::from_adapter_handle(&adapter_handle),
            handle
        );

        let credential = credential_fixture();
        let credential_id = PasskeyCredentialId::from_credential(&credential);
        assert_eq!(
            format!("{credential_id:?}"),
            "PasskeyCredentialId([redacted])"
        );
        assert_eq!(
            PasskeyCredentialId::from_bytes(credential.credential_id()),
            credential_id
        );
    }

    #[test]
    fn raw_ceremony_handles_are_redacted_and_hash_to_a_valid_digest() {
        let raw = RawPasskeyCeremonyHandle::generate();
        assert_eq!(format!("{raw:?}"), "RawPasskeyCeremonyHandle([REDACTED])");
        let hash = raw.hash();
        assert_eq!(
            hash.to_string()
                .parse::<StoredPasskeyCeremonyHandleHash>()
                .unwrap(),
            hash
        );
        let reparsed = raw
            .expose_to_browser()
            .parse::<RawPasskeyCeremonyHandle>()
            .unwrap();
        assert_eq!(reparsed.hash(), hash);
        assert!(
            "not a ceremony handle"
                .parse::<RawPasskeyCeremonyHandle>()
                .is_err()
        );
        assert!(
            URL_SAFE_NO_PAD
                .encode([0_u8; 31])
                .parse::<RawPasskeyCeremonyHandle>()
                .is_err()
        );
    }

    #[test]
    fn labels_trim_but_reject_blank_input() {
        assert_eq!(
            "  Laptop  ".parse::<PasskeyLabel>().unwrap().as_ref(),
            "Laptop"
        );
        assert!(" \t ".parse::<PasskeyLabel>().is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn created_users_receive_distinct_durable_handles(#[case] backend: Backend) {
        let env = backend.setup().await;
        let first = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let second = SeedUser::new()
            .seed(Arc::clone(&env.users()), env.write_scope().clone())
            .await
            .user_id;
        let first_handle = env.passkeys().user_handle(first).await.unwrap().unwrap();
        let second_handle = env.passkeys().user_handle(second).await.unwrap().unwrap();
        assert_ne!(first_handle, second_handle);
        assert_eq!(
            env.passkeys().user_for_handle(&first_handle).await.unwrap(),
            Some(first)
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn registration_ceremony_is_bound_to_its_purpose_and_claimed_once(
        #[case] backend: Backend,
    ) {
        use common::test_support::parse_session_label;
        use host::passkey::RelyingParty;
        use host::token;

        let env = backend.setup().await;
        let user_id = seed_user(&env).await;
        let sessions = env.sessions();
        let label = parse_session_label("Passkey test");
        let raw_session = crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(
                        async move { sessions.create_session(transaction, user_id, &label).await },
                    )
                })
                .await
                .unwrap(),
        );
        let user_handle = env.passkeys().user_handle(user_id).await.unwrap().unwrap();
        let base_url = "https://passkeys.example.test/".parse().unwrap();
        let (_, state) = RelyingParty::from_base_url(&base_url)
            .unwrap()
            .start_registration(
                &user_handle.adapter_handle().unwrap(),
                "account",
                "Account",
                &[],
            )
            .unwrap();
        let now = "2026-09-13T12:00:00Z".parse().unwrap();
        let expires_at = "2026-09-13T12:05:00Z".parse().unwrap();
        let handle = RawPasskeyCeremonyHandle::generate().hash();
        let ceremony = RegistrationCeremony {
            user_id,
            session_token_hash: token::hash(&raw_session).unwrap(),
            label: "Laptop".parse().unwrap(),
            origin: "https://passkeys.example.test".to_owned(),
            rp_id: "passkeys.example.test".to_owned(),
            state,
        };
        let passkeys = env.passkeys();
        let stored_handle = handle.clone();
        let stored_ceremony = ceremony.clone();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .create_registration_ceremony(
                                transaction,
                                &stored_handle,
                                &stored_ceremony,
                                expires_at,
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let expired = RawPasskeyCeremonyHandle::generate().hash();
        let passkeys = env.passkeys();
        let expired_handle = expired.clone();
        let expired_ceremony = ceremony.clone();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .create_registration_ceremony(
                                transaction,
                                &expired_handle,
                                &expired_ceremony,
                                now,
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let passkeys = env.passkeys();
        let expired_handle = expired.clone();
        let expired_claim = crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .claim_registration_ceremony(transaction, &expired_handle, now)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert!(expired_claim.is_none());
        assert!(
            env.base
                .pool()
                .execute("UPDATE passkey_registration_ceremonies SET purpose = 'authentication'")
                .await
                .is_err()
        );

        let first_scope = env.write_scope().clone();
        let first_passkeys = env.passkeys();
        let first_handle = handle.clone();
        let first = first_scope.run(move |transaction| {
            Box::pin(async move {
                first_passkeys
                    .claim_registration_ceremony(transaction, &first_handle, now)
                    .await
            })
        });
        let second_scope = env.write_scope().clone();
        let second_passkeys = env.passkeys();
        let second_handle = handle.clone();
        let second = second_scope.run(move |transaction| {
            Box::pin(async move {
                second_passkeys
                    .claim_registration_ceremony(transaction, &second_handle, now)
                    .await
            })
        });
        let (first, second) = tokio::join!(first, second);
        let claims = [
            crate::test_support::confirmed(first.unwrap()),
            crate::test_support::confirmed(second.unwrap()),
        ];
        assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
        assert_eq!(
            claims
                .into_iter()
                .flatten()
                .next()
                .expect("one claim")
                .user_id,
            user_id
        );
        let passkeys = env.passkeys();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move { passkeys.cleanup_ceremonies(transaction, now).await })
                })
                .await
                .unwrap(),
        );
        let remaining = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM passkey_registration_ceremonies")
                .fetch_one(pool)
                .await
                .unwrap()
        });
        assert_eq!(remaining, 0);
    }
    #[apply(backends)]
    #[tokio::test]
    async fn credentials_are_global_but_lookup_and_deletion_are_owner_scoped(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let owner = seed_user(&env).await;
        let other = seed_user(&env).await;
        let credential = insert_credential(&env, owner, "  Laptop  ").await;
        let id = PasskeyCredentialId::from_credential(&credential);
        let alternate = URL_SAFE_NO_PAD.encode([2_u8; 64]);
        let second = credential_fixture_with_id(&alternate);
        let stored_second = second.clone();
        let label = "Phone".parse().unwrap();
        let passkeys = env.passkeys();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .insert_credential(transaction, owner, &label, &stored_second)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let created_at = "2026-09-13T12:00:00Z".parse::<UtcInstant>().unwrap();
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE passkey_credentials SET created_at = $1 WHERE user_id = $2")
                .bind_storage(created_at)
                .bind_storage(owner)
                .execute(pool)
                .await
                .unwrap();
        });
        let listed = env.passkeys().list_credentials(owner).await.unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|credential| credential.id.clone())
                .collect::<Vec<_>>(),
            vec![PasskeyCredentialId::from_credential(&second), id.clone(),]
        );

        assert_eq!(
            env.passkeys()
                .credential(&id)
                .await
                .unwrap()
                .expect("credential exists")
                .label
                .as_ref(),
            "Laptop"
        );
        assert!(
            env.passkeys()
                .credential_for_user(other, &id)
                .await
                .unwrap()
                .is_none()
        );
        let passkeys = env.passkeys();
        let deleted_id = id.clone();
        let deleted = crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .delete_credential(transaction, other, &deleted_id)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert!(!deleted);
        assert!(env.passkeys().credential(&id).await.unwrap().is_some());

        let duplicate = credential_fixture();
        let label = "Phone".parse().unwrap();
        let passkeys = env.passkeys();
        let duplicate_error = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    passkeys
                        .insert_credential(transaction, other, &label, &duplicate)
                        .await
                })
            })
            .await;
        assert!(duplicate_error.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn credential_payload_identity_disagreement_is_rejected_on_read(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user_id = seed_user(&env).await;
        let credential = insert_credential(&env, user_id, "Laptop").await;
        let id = PasskeyCredentialId::from_credential(&credential);
        let alternate = URL_SAFE_NO_PAD.encode([1_u8; 64]);
        let corrupt_id =
            PasskeyCredentialId::from_credential(&credential_fixture_with_id(&alternate));
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(
                "UPDATE passkey_credentials SET credential_id = $1 WHERE credential_id = $2",
            )
            .bind_storage(corrupt_id)
            .bind_storage(id)
            .execute(pool)
            .await
            .unwrap();
        });
        let error = env.passkeys().list_credentials(user_id).await.unwrap_err();
        assert!(matches!(error, sqlx::Error::Decode(_)));
    }
    #[apply(backends)]
    #[tokio::test]
    async fn authentication_ceremonies_enforce_expiry_claim_once_and_cleanup(
        #[case] backend: Backend,
    ) {
        use host::passkey::RelyingParty;

        let env = backend.setup().await;
        let base_url = "https://passkeys.example.test/".parse().unwrap();
        let (_, state) = RelyingParty::from_base_url(&base_url)
            .unwrap()
            .start_authentication()
            .unwrap();
        let ceremony = AuthenticationCeremony {
            origin: "https://passkeys.example.test".to_owned(),
            rp_id: "passkeys.example.test".to_owned(),
            state,
        };
        let now = "2026-09-13T12:00:00Z".parse().unwrap();
        let expired = RawPasskeyCeremonyHandle::generate().hash();
        let live = RawPasskeyCeremonyHandle::generate().hash();
        let passkeys = env.passkeys();
        let expired_handle = expired.clone();
        let expired_ceremony = ceremony.clone();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .create_authentication_ceremony(
                                transaction,
                                &expired_handle,
                                &expired_ceremony,
                                now,
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let passkeys = env.passkeys();
        let live_handle = live.clone();
        let live_ceremony = ceremony.clone();
        let expires_at = "2026-09-13T12:05:00Z".parse().unwrap();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .create_authentication_ceremony(
                                transaction,
                                &live_handle,
                                &live_ceremony,
                                expires_at,
                            )
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let passkeys = env.passkeys();
        let expired_handle = expired.clone();
        let expired_claim = crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        passkeys
                            .claim_authentication_ceremony(transaction, &expired_handle, now)
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        assert!(expired_claim.is_none());
        assert!(
            env.base
                .pool()
                .execute("UPDATE passkey_authentication_ceremonies SET purpose = 'registration'")
                .await
                .is_err()
        );

        let first_scope = env.write_scope().clone();
        let first_passkeys = env.passkeys();
        let first_handle = live.clone();
        let first = first_scope.run(move |transaction| {
            Box::pin(async move {
                first_passkeys
                    .claim_authentication_ceremony(transaction, &first_handle, now)
                    .await
            })
        });
        let second_scope = env.write_scope().clone();
        let second_passkeys = env.passkeys();
        let second_handle = live.clone();
        let second = second_scope.run(move |transaction| {
            Box::pin(async move {
                second_passkeys
                    .claim_authentication_ceremony(transaction, &second_handle, now)
                    .await
            })
        });
        let (first, second) = tokio::join!(first, second);
        let claims = [
            crate::test_support::confirmed(first.unwrap()),
            crate::test_support::confirmed(second.unwrap()),
        ];
        assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
        let passkeys = env.passkeys();
        crate::test_support::confirmed(
            env.write_scope()
                .run(move |transaction| {
                    Box::pin(async move { passkeys.cleanup_ceremonies(transaction, now).await })
                })
                .await
                .unwrap(),
        );
        let remaining = crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM passkey_authentication_ceremonies")
                .fetch_one(pool)
                .await
                .unwrap()
        });
        assert_eq!(remaining, 0);
    }
}

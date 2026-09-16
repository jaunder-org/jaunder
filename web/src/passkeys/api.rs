//! Typed Passkey ceremony and credential-management server functions.
//!
//! Ceremony handles are accepted only at this boundary, immediately hashed for
//! storage, and are deliberately absent from errors and telemetry.

use crate::auth;
use crate::error::WebResult;
use common::{MutationOutcome, password::ProfferedPassword};
use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[cfg(feature = "server")]
use {
    crate::error::{InternalError, from_write_scope_error},
    common::session_label::SessionLabel,
    host::{
        metrics,
        passkey::{
            AuthenticationResponse, CounterOutcome, RegistrationResponse, RelyingParty,
            webauthn_identity,
        },
        password::Password,
    },
    jiff::ToSpan,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{
        AuthenticationCeremony, PasskeyCredentialId, PasskeyStorage, PasskeyUserHandle,
        RawPasskeyCeremonyHandle, RegistrationCeremony, SessionStorage, SiteConfigStorage,
        UserStorage, WriteScope,
        account_mutations::{
            delete_passkey_and_revoke_other_sessions, finalize_passkey_authentication,
        },
    },
};

/// Browser-facing registration ceremony payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistrationStart {
    pub handle: String,
    pub request: serde_json::Value,
}

/// Browser-facing discoverable authentication ceremony payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthenticationStart {
    pub handle: String,
    pub request: serde_json::Value,
}

/// A credential visible to its owner.  The persisted public-key material never
/// crosses this API boundary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialInfo {
    pub id: String,
    pub label: String,
    pub created_at: common::time::UtcInstant,
    pub last_used_at: Option<common::time::UtcInstant>,
}

#[cfg(feature = "server")]
const CEREMONY_MINUTES: i64 = 5;

/// A nonblank, presentation-only credential label accepted from the browser.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BrowserPasskeyLabel(String);

/// Bounded wire-decode reason for a browser-supplied passkey label.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum InvalidBrowserPasskeyLabel {
    #[error("passkey label must not be blank")]
    Blank,
}

impl InvalidBrowserPasskeyLabel {
    /// Stable validation feedback safe to return across the wire boundary.
    #[must_use]
    pub fn user_message(self) -> &'static str {
        match self {
            Self::Blank => "passkey label must not be blank",
        }
    }

    /// Bounded reason code for decode-failure telemetry.
    #[must_use]
    pub fn telemetry_code(self) -> &'static str {
        match self {
            Self::Blank => "blank_passkey_label",
        }
    }
}

impl TryFrom<String> for BrowserPasskeyLabel {
    type Error = InvalidBrowserPasskeyLabel;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = value.trim();
        if value.is_empty() {
            return Err(InvalidBrowserPasskeyLabel::Blank);
        }
        Ok(Self(value.to_owned()))
    }
}

impl FromStr for BrowserPasskeyLabel {
    type Err = InvalidBrowserPasskeyLabel;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl From<BrowserPasskeyLabel> for String {
    fn from(value: BrowserPasskeyLabel) -> Self {
        value.0
    }
}

#[cfg(feature = "server")]
impl BrowserPasskeyLabel {
    fn storage_label(&self) -> Result<storage::PasskeyLabel, InternalError> {
        self.0
            .parse()
            .map_err(|_| InternalError::validation("invalid passkey label"))
    }
}
/// A one-time registration ceremony handle validated during wire decoding.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CeremonyHandle(String);

impl TryFrom<String> for CeremonyHandle {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        #[cfg(feature = "server")]
        RawPasskeyCeremonyHandle::from_str(&value)
            .map_err(|_| "invalid ceremony handle".to_owned())?;
        Ok(Self(value))
    }
}

impl From<CeremonyHandle> for String {
    fn from(value: CeremonyHandle) -> Self {
        value.0
    }
}

/// An owner-visible credential identity.  Storage keeps its corresponding
/// identifier non-serializable, so this is the explicit wire-only wrapper.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BrowserCredentialId(String);

impl TryFrom<String> for BrowserCredentialId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        #[cfg(feature = "server")]
        PasskeyCredentialId::from_str(&value)
            .map_err(|_| "invalid credential identifier".to_owned())?;
        Ok(Self(value))
    }
}

impl From<BrowserCredentialId> for String {
    fn from(value: BrowserCredentialId) -> Self {
        value.0
    }
}

#[cfg(feature = "server")]
impl BrowserCredentialId {
    fn storage_id(&self) -> Result<PasskeyCredentialId, InternalError> {
        PasskeyCredentialId::from_str(&self.0)
            .map_err(|_| InternalError::validation("invalid passkey"))
    }
}

#[cfg(feature = "server")]
fn ceremony_expiry() -> Result<common::time::UtcInstant, InternalError> {
    common::time::UtcInstant::now()
        .value()
        .saturating_add(CEREMONY_MINUTES.minutes())
        .map(common::time::UtcInstant::from)
        .map_err(|_| InternalError::server_message("passkey ceremony expiry failed"))
}

#[cfg(feature = "server")]
async fn relying_party(
    config: &Arc<dyn SiteConfigStorage>,
) -> Result<(RelyingParty, String, String), InternalError> {
    let base_url = config
        .get_identity()
        .await?
        .base_url
        .ok_or_else(|| InternalError::validation("passkeys are unavailable"))?;
    let (origin, rp_id) = webauthn_identity(&base_url)
        .map_err(|_| InternalError::validation("passkeys are unavailable"))?;
    let rp = RelyingParty::from_base_url(&base_url)
        .map_err(|_| InternalError::validation("passkeys are unavailable"))?;
    Ok((rp, origin, rp_id))
}

/// Returns whether this deployment has a usable relying-party identity.
#[macros::server]
pub async fn availability() -> WebResult<bool> {
    let config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let Some(base_url) = config.get_identity().await?.base_url else {
        return Ok(false);
    };
    Ok(webauthn_identity(&base_url).is_ok() && RelyingParty::from_base_url(&base_url).is_ok())
}

/// Starts a cookie-session-bound registration ceremony after verifying the
/// caller's current password.
#[macros::server(skip_all)]
pub async fn start_registration(
    label: BrowserPasskeyLabel,
    password: ProfferedPassword,
) -> WebResult<MutationOutcome<RegistrationStart>> {
    let auth = auth::require_cookie_auth().await?;
    let password = Password::try_from(password)?;
    let label = label.storage_label()?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    users
        .prepare_authentication(&auth.username, &password)
        .await
        .map_err(InternalError::from)?;
    let passkeys = expect_context::<Arc<dyn PasskeyStorage>>();
    let config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let user = users
        .get_user(auth.user_id)
        .await?
        .ok_or_else(|| InternalError::unauthorized("invalid session"))?;
    let handle = passkeys
        .user_handle(auth.user_id)
        .await?
        .ok_or_else(|| InternalError::server_message("missing passkey handle"))?;
    let existing = passkeys.list_credentials(auth.user_id).await?;
    let (rp, origin, rp_id) = relying_party(&config).await?;
    let (request, state) = rp
        .start_registration(
            &handle
                .adapter_handle()
                .map_err(|_| InternalError::server_message("invalid passkey handle"))?,
            user.username.as_ref(),
            user.display_name
                .as_ref()
                .map_or(user.username.as_ref(), |name| name.as_ref()),
            &existing
                .into_iter()
                .map(|credential| credential.credential)
                .collect::<Vec<_>>(),
        )
        .map_err(|_| InternalError::validation("passkey registration could not start"))?;
    let raw = RawPasskeyCeremonyHandle::generate();
    let response = RegistrationStart {
        handle: raw.expose_to_browser().to_owned(),
        request: serde_json::to_value(request)
            .map_err(|_| InternalError::server_message("passkey request encoding failed"))?,
    };
    let digest = raw.hash();
    write_scope
        .run(move |transaction| {
            let passkeys = passkeys.clone();
            Box::pin(async move {
                passkeys
                    .create_registration_ceremony(
                        transaction,
                        &digest,
                        &RegistrationCeremony {
                            user_id: auth.user_id,
                            session_token_hash: auth.token_hash,
                            label,
                            origin,
                            rp_id,
                            state,
                        },
                        ceremony_expiry()?,
                    )
                    .await
                    .map(|()| response)
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Completes the cookie-session-bound registration ceremony exactly once.
#[macros::server(input = Json, skip_all)]
pub async fn finish_registration(
    handle: CeremonyHandle,
    response: serde_json::Value,
) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_cookie_auth().await?;
    let raw = RawPasskeyCeremonyHandle::from_str(&handle.0)
        .map_err(|_| InternalError::validation("invalid ceremony"))?;
    let passkeys = expect_context::<Arc<dyn PasskeyStorage>>();
    let config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let digest = raw.hash();
    let claim_passkeys = passkeys.clone();
    let ceremony = match write_scope
        .run(move |transaction| {
            let passkeys = claim_passkeys.clone();
            Box::pin(async move {
                passkeys
                    .claim_registration_ceremony(
                        transaction,
                        &digest,
                        common::time::UtcInstant::now(),
                    )
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)?
    {
        MutationOutcome::Confirmed(Some(ceremony))
            if ceremony.user_id == auth.user_id
                && ceremony.session_token_hash == auth.token_hash =>
        {
            ceremony
        }
        MutationOutcome::Confirmed(None | Some(_)) | MutationOutcome::CommitIndeterminate(_) => {
            return Err(InternalError::validation("invalid ceremony"));
        }
    };
    write_scope
        .run(move |transaction| {
            let passkeys = passkeys.clone();
            let config = config.clone();
            Box::pin(async move {
                passkeys
                    .lock_rp_host(transaction)
                    .await
                    .map_err(InternalError::storage)?;
                let (rp, origin, rp_id) = relying_party(&config).await?;
                if ceremony.origin != origin || ceremony.rp_id != rp_id {
                    return Err(InternalError::validation("invalid ceremony"));
                } // cov:ignore: The invalid-ceremony return is covered; LLVM attributes the closing branch edge separately.
                // cov:ignore-start: Valid registration responses require browser-generated WebAuthn cryptographic material; Chromium virtual-authenticator E2E covers verification and durable credential insertion.
                let credential = rp
                    .finish_registration(RegistrationResponse::from_json(response), &ceremony.state)
                    .map_err(|_| InternalError::validation("passkey registration failed"))?;
                passkeys
                    .insert_credential(transaction, auth.user_id, &ceremony.label, &credential)
                    .await
                    .map_err(InternalError::storage)
                // cov:ignore-stop
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Starts a public discoverable authentication ceremony.
#[macros::server]
pub async fn start_authentication() -> WebResult<MutationOutcome<AuthenticationStart>> {
    let passkeys = expect_context::<Arc<dyn PasskeyStorage>>();
    let config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let (rp, origin, rp_id) = relying_party(&config).await?;
    let (request, state) = rp
        .start_authentication()
        .map_err(|_| InternalError::validation("passkey authentication could not start"))?;
    let raw = RawPasskeyCeremonyHandle::generate();
    let result = AuthenticationStart {
        handle: raw.expose_to_browser().to_owned(),
        request: serde_json::to_value(request)
            .map_err(|_| InternalError::server_message("passkey request encoding failed"))?,
    };
    let digest = raw.hash();
    write_scope
        .run(move |transaction| {
            let passkeys = passkeys.clone();
            Box::pin(async move {
                passkeys
                    .create_authentication_ceremony(
                        transaction,
                        &digest,
                        &AuthenticationCeremony {
                            origin,
                            rp_id,
                            state,
                        },
                        ceremony_expiry()?,
                    )
                    .await
                    .map(|()| result)
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Completes a public discoverable authentication ceremony and establishes the
/// ordinary browser session cookie; no raw token is returned in the body.
#[macros::server(input = Json, skip_all)]
pub async fn finish_authentication(
    handle: String,
    response: serde_json::Value,
) -> WebResult<MutationOutcome<auth::SessionUser>> {
    let raw = RawPasskeyCeremonyHandle::from_str(&handle)
        .map_err(|_| InternalError::validation("authentication failed"))?;
    let passkeys = expect_context::<Arc<dyn PasskeyStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    let config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let digest = raw.hash();
    let claim_passkeys = passkeys.clone();
    let ceremony = match write_scope
        .run(move |transaction| {
            let passkeys = claim_passkeys.clone();
            Box::pin(async move {
                passkeys
                    .claim_authentication_ceremony(
                        transaction,
                        &digest,
                        common::time::UtcInstant::now(),
                    )
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)?
    {
        MutationOutcome::Confirmed(Some(ceremony)) => ceremony,
        MutationOutcome::Confirmed(None) | MutationOutcome::CommitIndeterminate(_) => {
            return Err(InternalError::validation("authentication failed"));
        }
    };
    let outcome = write_scope
        .run(move |transaction| {
            let passkeys = passkeys.clone();
            let users = users.clone();
            let sessions = sessions.clone();
            let config = config.clone();
            Box::pin(async move {
                passkeys
                    .lock_rp_host(transaction)
                    .await
                    .map_err(InternalError::storage)?;
                let base_url = config
                    .get_identity()
                    .await
                    .map_err(InternalError::storage)?
                    .base_url
                    .ok_or_else(|| InternalError::validation("authentication failed"))?;
                let (origin, rp_id) = webauthn_identity(&base_url).map_err(|error| {
                    InternalError::validation_source("authentication failed", error)
                        .with_context("passkey.stage", "configuration")
                })?;
                // cov:ignore-start: `webauthn_identity` already accepted `base_url`; this redundant typed constructor error is unreachable by construction.
                let rp = RelyingParty::from_base_url(&base_url).map_err(|error| {
                    InternalError::validation_source("authentication failed", error)
                        .with_context("passkey.stage", "configuration")
                })?;
                // cov:ignore-stop
                if ceremony.origin != origin || ceremony.rp_id != rp_id {
                    return Err(InternalError::validation("authentication failed"));
                }
                let assertion = AuthenticationResponse::from_json(response);
                let discovered = rp.identify(&assertion).map_err(|error| {
                    InternalError::validation_source("authentication failed", error)
                        .with_context("passkey.stage", "identify")
                })?;
                let user_id = passkeys
                    .user_for_handle(&PasskeyUserHandle::from_adapter_handle(
                        &discovered.user_handle,
                    ))
                    .await
                    .map_err(InternalError::storage)?
                    .ok_or_else(|| InternalError::validation("authentication failed"))?;
                let credential_id = PasskeyCredentialId::from_bytes(&discovered.credential_id);
                let stored = passkeys
                    .credential_for_authentication(transaction, &credential_id)
                    .await
                    .map_err(InternalError::storage)?
                    .filter(|credential| credential.user_id == user_id)
                    .ok_or_else(|| InternalError::validation("authentication failed"))?;
                // cov:ignore-start: Signed assertion verification and its post-verification session and user lifecycle require browser-generated WebAuthn cryptographic material; Chromium virtual-authenticator E2E covers counter anomalies and lifecycle.
                let verified = rp
                    .finish_authentication(assertion, ceremony.state, &stored.credential)
                    .map_err(|error| {
                        InternalError::validation_source("authentication failed", error)
                            .with_context("passkey.stage", "verify")
                    })?;
                if matches!(verified.counter, CounterOutcome::RetainHighWater { .. }) {
                    metrics::passkey_counter_anomaly();
                }
                let token = finalize_passkey_authentication(
                    transaction,
                    passkeys.as_ref(),
                    sessions.as_ref(),
                    user_id,
                    &verified.credential,
                    &SessionLabel::from_lossy("Passkey"),
                )
                .await
                .map_err(InternalError::storage)?;
                let user = users
                    .get_user(user_id)
                    .await?
                    .ok_or_else(|| InternalError::validation("authentication failed"))?;
                Ok((
                    token,
                    auth::SessionUser {
                        username: user.username,
                        is_operator: user.is_operator.is_operator(),
                    },
                ))
                // cov:ignore-stop
            })
        })
        .await
        .map_err(from_write_scope_error)?;
    // cov:ignore-start: The post-verification session cookie outcome requires a signed browser assertion; Chromium virtual-authenticator E2E covers its lifecycle.
    match outcome {
        MutationOutcome::Confirmed((token, user)) => {
            auth::set_session_cookie(&token);
            Ok(MutationOutcome::Confirmed(user))
        }
        MutationOutcome::CommitIndeterminate((token, user)) => {
            auth::set_session_cookie(&token);
            Ok(MutationOutcome::CommitIndeterminate(user))
        }
    }
    // cov:ignore-stop
}

/// Lists the caller's registered credentials.
#[macros::server]
pub async fn list() -> WebResult<Vec<CredentialInfo>> {
    let auth = auth::require_cookie_auth().await?;
    let passkeys = expect_context::<Arc<dyn PasskeyStorage>>();
    Ok(passkeys
        .list_credentials(auth.user_id)
        .await?
        .into_iter()
        .map(|credential| CredentialInfo {
            id: credential.id.expose_to_browser().to_owned(),
            label: credential.label.to_string(),
            created_at: credential.created_at,
            last_used_at: credential.last_used_at,
        })
        .collect())
}

/// Deletes one owned credential after verifying the current password, preserving
/// the current cookie session and revoking its siblings atomically.
#[macros::server(skip_all)]
pub async fn delete(
    credential_id: BrowserCredentialId,
    password: ProfferedPassword,
) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_cookie_auth().await?;
    let password = Password::try_from(password)?;
    let credential_id = credential_id.storage_id()?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    users
        .prepare_authentication(&auth.username, &password)
        .await
        .map_err(InternalError::from)?;
    let passkeys = expect_context::<Arc<dyn PasskeyStorage>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            let passkeys = passkeys.clone();
            let sessions = sessions.clone();
            Box::pin(async move {
                if !delete_passkey_and_revoke_other_sessions(
                    transaction,
                    passkeys.as_ref(),
                    sessions.as_ref(),
                    auth.user_id,
                    &credential_id,
                    &auth.token_hash,
                )
                .await
                .map_err(InternalError::storage)?
                {
                    return Err(InternalError::unauthorized("passkey deletion rejected"));
                }
                Ok(())
            })
        })
        .await
        .map_err(from_write_scope_error)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "server")]
    use super::BrowserCredentialId;
    use super::{BrowserPasskeyLabel, CeremonyHandle, InvalidBrowserPasskeyLabel};
    #[cfg(feature = "server")]
    use super::{finish_authentication, start_authentication};
    use std::str::FromStr;
    #[cfg(feature = "server")]
    use storage::{PasskeyCredentialId, RawPasskeyCeremonyHandle};
    #[cfg(feature = "server")]
    use {
        crate::error::WebError,
        common::{MutationOutcome, site::SiteIdentity, test_support::parse_url},
        leptos::prelude::{Owner, provide_context},
        std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        storage::{
            AuthenticationCeremony, MockPasskeyStorage, MockSessionStorage, MockSiteConfigStorage,
            MockUserStorage, PasskeyStorage, SessionStorage, SiteConfigStorage, UserStorage,
            test_support::mock_write_scope,
        },
    };

    #[test]
    fn browser_passkey_label_decode_error_has_safe_bounded_surfaces() {
        let error =
            BrowserPasskeyLabel::try_from(" \n ".to_owned()).expect_err("blank label is rejected");
        assert!(matches!(error, InvalidBrowserPasskeyLabel::Blank));
        assert_eq!(error.user_message(), "passkey label must not be blank");
        assert_eq!(error.telemetry_code(), "blank_passkey_label");
    }

    #[test]
    fn browser_passkey_label_from_str_trims_and_round_trips() {
        let label = BrowserPasskeyLabel::from_str("  My passkey \n").expect("valid passkey label");
        assert_eq!(String::from(label), "My passkey");
    }

    #[test]
    fn ceremony_handle_rejects_noncanonical_wire_values() {
        assert!(CeremonyHandle::try_from("not-a-handle".to_owned()).is_err());
    }

    #[cfg(feature = "server")]
    #[test]
    fn ceremony_handle_from_valid_raw_value_round_trips() {
        let raw = RawPasskeyCeremonyHandle::generate();
        let handle = CeremonyHandle::try_from(raw.expose_to_browser().to_owned())
            .expect("generated raw ceremony handle is valid");
        assert_eq!(String::from(handle), raw.expose_to_browser());
    }

    #[cfg(feature = "server")]
    #[test]
    fn browser_credential_id_from_valid_id_round_trips() {
        let id = PasskeyCredentialId::from_bytes(&[1, 2, 3]);
        let browser_id = BrowserCredentialId::try_from(id.expose_to_browser().to_owned())
            .expect("generated credential identifier is valid");
        assert_eq!(String::from(browser_id), id.expose_to_browser());
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn authentication_rejects_same_host_origin_drift_before_assertion_parsing() {
        let owner = Owner::new();
        owner.set();
        let original_url = parse_url("https://passkeys.example.test/");
        let changed_url = parse_url("https://passkeys.example.test:8443/");
        let identities = [
            SiteIdentity {
                title: "Jaunder".parse().expect("valid fixture title"),
                tagline: None,
                base_url: Some(original_url),
            },
            SiteIdentity {
                title: "Jaunder".parse().expect("valid fixture title"),
                tagline: None,
                base_url: Some(changed_url),
            },
        ];
        let calls = AtomicUsize::new(0);
        let mut config = MockSiteConfigStorage::new();
        config.expect_get_identity().times(2).returning(move || {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            Ok(identities
                .get(call)
                .cloned()
                .expect("authentication reads identity exactly twice"))
        });

        let captured_ceremony = Arc::new(Mutex::new(None::<AuthenticationCeremony>));
        let create_ceremony = Arc::clone(&captured_ceremony);
        let claim_ceremony = Arc::clone(&captured_ceremony);
        let mut passkeys = MockPasskeyStorage::new();
        passkeys
            .expect_create_authentication_ceremony()
            .times(1)
            .returning(move |_, _, ceremony, _| {
                let captured_ceremony = Arc::clone(&create_ceremony);
                let ceremony = ceremony.clone();
                Box::pin(async move {
                    *captured_ceremony.lock().expect("ceremony capture lock") = Some(ceremony);
                    Ok(())
                })
            });
        passkeys
            .expect_claim_authentication_ceremony()
            .times(1)
            .returning(move |_, _, _| {
                let captured_ceremony = Arc::clone(&claim_ceremony);
                Box::pin(async move {
                    Ok(captured_ceremony
                        .lock()
                        .expect("ceremony capture lock")
                        .take())
                })
            });
        passkeys
            .expect_lock_rp_host()
            .times(1)
            .returning(|_| Box::pin(async { Ok(()) }));
        provide_context(Arc::new(passkeys) as Arc<dyn PasskeyStorage>);
        provide_context(Arc::new(config) as Arc<dyn SiteConfigStorage>);
        provide_context(Arc::new(MockUserStorage::new()) as Arc<dyn UserStorage>);
        provide_context(Arc::new(MockSessionStorage::new()) as Arc<dyn SessionStorage>);
        provide_context(mock_write_scope());

        let MutationOutcome::Confirmed(start) = start_authentication()
            .await
            .expect("authentication starts before configuration changes")
        else {
            unreachable!("mock write scope always confirms the authentication start");
        };

        assert_eq!(
            finish_authentication(start.handle, serde_json::json!({"not": "an assertion"})).await,
            Err(WebError::validation("authentication failed")),
            "origin drift must fail closed before parsing the deliberately malformed assertion"
        );
    }
}

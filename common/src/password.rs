use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

const MIN_LENGTH: usize = 8;

/// A validated plaintext password with a minimum length of [`MIN_LENGTH`].
///
/// Constructed via [`FromStr`]; passwords that are too short are rejected at
/// the boundary. Interior code works only with [`Password`] values and never
/// with raw strings.
///
/// Adopts the [`StrNewtype`] `secret` surface (ADR-0063 §2): a redacting `Debug`
/// and borrowed `AsRef<str>` access for hashing, with no `Display`, serde, or
/// owned-`String` escape hatch — so a `Password` cannot be rendered, serialised,
/// or leaked. The `macros` crate is the authoritative list of what `secret` emits.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret)]
pub struct Password(String);

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password must be at least {MIN_LENGTH} characters")]
    PasswordTooShort,
    #[error("hashing failed: {0}")]
    HashingFailed(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
}

impl FromStr for Password {
    type Err = PasswordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_password_shape(s)?;
        Ok(Password(s.to_owned()))
    }
}

/// The shared shape invariant for a plaintext password: at least [`MIN_LENGTH`]
/// characters. Both [`Password`] and [`ProfferedPassword`] delegate to it, so the
/// inbound wire type and the domain type cannot drift (mirrors invite codes'
/// `common::token::validate_shape`).
fn validate_password_shape(s: &str) -> Result<(), PasswordError> {
    if s.len() < MIN_LENGTH {
        return Err(PasswordError::PasswordTooShort);
    }
    Ok(())
}

/// A raw plaintext password as **submitted by a client** during registration,
/// login, or password-reset confirmation.
///
/// The serde-capable _inbound_ twin of the secret [`Password`], per ADR-0063's
/// inbound-secret variant (`#[str_newtype(secret, serde)]`): redacting `Debug`,
/// `AsRef<str>`, `TryFrom<String>`, and the validating serde bridge — but no
/// `Display`/`Deref`/owned-`String`. It exists only to be validated (client-side
/// per ADR-0065, and again on the wire at deserialize), travel client→server, and
/// be converted into [`Password`]. A `proffered-secret` xtask gate pins it to
/// `#[server]` parameter positions, so a plaintext password can never be rendered
/// or returned to a client.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, serde)]
pub struct ProfferedPassword(String);

impl FromStr for ProfferedPassword {
    type Err = PasswordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_password_shape(s)?;
        Ok(ProfferedPassword(s.to_owned()))
    }
}

impl TryFrom<ProfferedPassword> for Password {
    type Error = PasswordError;

    /// Converts a client-submitted password into the domain type. `ProfferedPassword`
    /// was already validated at construction, so this cannot actually fail — but it
    /// re-runs the shared validator rather than relying on that (no infallible
    /// cross-type constructor). Mirrors `InviteCode: TryFrom<ProfferedInviteCode>`.
    fn try_from(p: ProfferedPassword) -> Result<Self, Self::Error> {
        p.as_ref().parse()
    }
}

impl Password {
    /// Hashes the password using Argon2id with default parameters.
    ///
    /// This is a CPU-intensive operation and should be called from a blocking
    /// context (e.g., via [`tokio::task::spawn_blocking`]).
    ///
    /// # Errors
    ///
    /// Returns `Err` if Argon2 hashing fails.
    pub fn hash(&self) -> Result<String, PasswordError> {
        use argon2::{
            password_hash::{rand_core::OsRng, SaltString},
            PasswordHasher,
        };

        let salt = SaltString::generate(&mut OsRng);

        // Production uses the crate defaults (m=19456, t=2). Under `cheap-kdf`
        // (test builds only) use the minimum memory cost so the suite is not
        // dominated by KDF time. `verify()` derives cost from the stored hash, so
        // it needs no branch.
        #[cfg(feature = "cheap-kdf")]
        let hasher = {
            use argon2::{Algorithm, Argon2, Params, Version};
            let params = Params::new(Params::MIN_M_COST, 1, 1, None)
                .map_err(|e| PasswordError::HashingFailed(e.to_string()))?;
            Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        };
        #[cfg(not(feature = "cheap-kdf"))]
        let hasher = argon2::Argon2::default();

        hasher
            .hash_password(self.0.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| PasswordError::HashingFailed(e.to_string()))
    }

    /// Verifies the password against a stored Argon2 hash.
    ///
    /// This is a CPU-intensive operation and should be called from a blocking
    /// context (e.g., via [`tokio::task::spawn_blocking`]).
    ///
    /// # Errors
    ///
    /// Returns `Err` if Argon2 verification fails (e.g., the hash string is malformed).
    pub fn verify(&self, hash: &str) -> Result<bool, PasswordError> {
        use argon2::{Argon2, PasswordHash, PasswordVerifier};

        let parsed = PasswordHash::new(hash)
            .map_err(|e| PasswordError::VerificationFailed(e.to_string()))?;
        match Argon2::default().verify_password(self.0.as_bytes(), &parsed) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(e) => Err(PasswordError::VerificationFailed(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_accepts_minimum_length() {
        assert!("12345678".parse::<Password>().is_ok());
        assert!("a longer passphrase".parse::<Password>().is_ok());
    }

    #[test]
    fn password_rejects_too_short() {
        assert!("".parse::<Password>().is_err());
        assert!("short".parse::<Password>().is_err());
        assert!("1234567".parse::<Password>().is_err());
    }

    #[test]
    fn proffered_from_str_valid_and_invalid() {
        assert!("12345678".parse::<ProfferedPassword>().is_ok());
        assert!("short".parse::<ProfferedPassword>().is_err());
        assert!("".parse::<ProfferedPassword>().is_err());
    }

    #[test]
    fn proffered_serde_roundtrips_and_validates_on_the_wire() {
        let p: ProfferedPassword = "password123".parse().unwrap();
        assert_eq!(serde_json::to_string(&p).unwrap(), "\"password123\"");
        let back: ProfferedPassword = serde_json::from_str("\"password123\"").unwrap();
        assert_eq!(back.as_ref(), "password123");
        // Deserialize routes through the shared shape validator, so a too-short
        // password is rejected on the wire.
        assert!(serde_json::from_str::<ProfferedPassword>("\"short\"").is_err());
    }

    #[test]
    fn proffered_debug_is_redacted() {
        let raw = "supersecret123";
        let p: ProfferedPassword = raw.parse().unwrap();
        let out = format!("{p:?}");
        assert!(!out.contains(raw));
        assert_eq!(out, "ProfferedPassword([redacted])");
    }

    #[test]
    fn proffered_converts_into_password() {
        let p: ProfferedPassword = "password123".parse().unwrap();
        let pw = Password::try_from(p).expect("valid proffered password converts");
        assert_eq!(pw.as_ref(), "password123");
    }

    #[test]
    fn debug_does_not_expose_value() {
        let val = "a".repeat(10);
        let p: Password = val.parse().unwrap();
        let debug_output = format!("{p:?}");
        assert!(!debug_output.contains(&val));
        assert_eq!(debug_output, "Password([redacted])");
    }

    #[test]
    fn as_ref_returns_original_value() {
        let raw = "correct horse battery staple";
        let p: Password = raw.parse().expect("password meets minimum length");
        assert_eq!(p.as_ref(), raw);
    }

    #[test]
    fn hash_and_verify_roundtrip() {
        let val = "a".repeat(10);
        let p: Password = val.parse().unwrap();
        let hash = p.hash().expect("hashing should succeed");
        assert!(p.verify(&hash).expect("verification should succeed"));
    }

    #[test]
    fn production_params_roundtrip_regardless_of_feature() {
        // Guards prod-strength Argon2 even when the workspace test build turns on
        // cheap-kdf: hash with explicit production params and verify.
        use argon2::{
            password_hash::{rand_core::OsRng, SaltString},
            Argon2, PasswordHasher,
        };
        let p: Password = "a".repeat(10).parse().unwrap();
        let salt = SaltString::generate(&mut OsRng);
        let prod_hash = Argon2::default()
            .hash_password(p.as_ref().as_bytes(), &salt)
            .unwrap()
            .to_string();
        assert!(
            prod_hash.contains("m=19456"),
            "prod params must be Argon2 default"
        );
        assert!(p.verify(&prod_hash).unwrap());
    }

    #[test]
    fn verify_rejects_wrong_password() {
        let v1 = "a".repeat(10);
        let v2 = "b".repeat(10);
        let p1: Password = v1.parse().unwrap();
        let p2: Password = v2.parse().unwrap();
        let hash = p1.hash().unwrap();
        assert!(!p2
            .verify(&hash)
            .expect("verification should return false, not error"));
    }

    #[test]
    fn verify_rejects_invalid_hash() {
        let val = "c".repeat(10);
        let p: Password = val.parse().unwrap();
        assert!(p.verify("not a valid argon2 hash").is_err());
    }

    #[test]
    fn verify_returns_error_for_non_password_argon2_failure() {
        // v=1 is not a supported argon2 version (only 16 and 19 are valid).
        // PasswordHash::new parses it; verify_password returns Error::Version,
        // which is not Error::Password, so the Err(e) arm in verify() is hit.
        let p: Password = "password1".parse().expect("minimum length");
        let hash =
            "$argon2id$v=1$m=65536,t=2,p=1$c29tZXNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let result = p.verify(hash);
        assert!(
            matches!(result.unwrap_err(), PasswordError::VerificationFailed(_)),
            "non-Password argon2 error must return VerificationFailed"
        );
    }
}

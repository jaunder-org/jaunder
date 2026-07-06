use std::{fmt, str::FromStr};

use thiserror::Error;

const MIN_LENGTH: usize = 8;

/// A validated plaintext password with a minimum length of [`MIN_LENGTH`].
///
/// Constructed via [`FromStr`]; passwords that are too short are rejected at
/// the boundary. Interior code works only with [`Password`] values and never
/// with raw strings.
///
/// [`Display`] is intentionally not implemented to prevent passwords from
/// being accidentally logged or serialised.
#[derive(Clone)]
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
        if s.len() < MIN_LENGTH {
            return Err(PasswordError::PasswordTooShort);
        }
        Ok(Password(s.to_owned()))
    }
}

impl Password {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

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

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password([redacted])")
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
    fn debug_does_not_expose_value() {
        let val = "a".repeat(10);
        let p: Password = val.parse().unwrap();
        let debug_output = format!("{p:?}");
        assert!(!debug_output.contains(&val));
        assert_eq!(debug_output, "Password([redacted])");
    }

    #[test]
    fn as_str_returns_original_value() {
        let raw = "correct horse battery staple";
        let p: Password = raw.parse().expect("password meets minimum length");
        assert_eq!(p.as_str(), raw);
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
            .hash_password(p.as_str().as_bytes(), &salt)
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

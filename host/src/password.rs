use std::str::FromStr;

use common::password::{self, InvalidPassword, ProfferedPassword};
use macros::StrNewtype;
use thiserror::Error;

/// A validated server-side plaintext password.
///
/// Constructed from a [`ProfferedPassword`] after reusing its shared input-shape
/// validation. Its secret newtype surface permits only borrowed access for hashing.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret)]
pub struct Password(String);

impl FromStr for Password {
    type Err = InvalidPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        password::validate_password_shape(s)?;
        Ok(Self(s.to_owned()))
    }
}

impl TryFrom<ProfferedPassword> for Password {
    type Error = InvalidPassword;

    fn try_from(password: ProfferedPassword) -> Result<Self, Self::Error> {
        password.as_ref().parse()
    }
}

/// Errors from host-side Argon2 password hashing and verification.
#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("hashing failed: {0}")]
    HashingFailed(#[source] argon2::password_hash::Error),
    #[error("verification failed: {0}")]
    VerificationFailed(#[source] argon2::password_hash::Error),
}

/// Hashes `password` with Argon2id's default parameters.
///
/// This CPU-intensive operation belongs in a blocking context such as
/// [`tokio::task::spawn_blocking`].
///
/// # Errors
///
/// Returns [`PasswordError::HashingFailed`] when Argon2 cannot create a hash.
pub fn hash(password: &Password) -> Result<String, PasswordError> {
    hash_with(password, hash_operation)
}

fn hash_with(password: &Password, operation: HashOperation) -> Result<String, PasswordError> {
    operation(password).map_err(PasswordError::HashingFailed)
}

/// Verifies `password` against a stored Argon2 hash.
///
/// This CPU-intensive operation belongs in a blocking context such as
/// [`tokio::task::spawn_blocking`]. A mismatch returns `Ok(false)`; malformed or
/// otherwise unusable hashes retain their Argon2 failure source.
///
/// # Errors
///
/// Returns [`PasswordError::VerificationFailed`] when Argon2 cannot parse or
/// verify the stored hash; a password mismatch is `Ok(false)`.
pub fn verify(password: &Password, hash: &str) -> Result<bool, PasswordError> {
    verify_with(password, hash, verify_operation)
}

fn verify_with(
    password: &Password,
    hash: &str,
    operation: VerifyOperation,
) -> Result<bool, PasswordError> {
    match operation(password, hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(error) => Err(PasswordError::VerificationFailed(error)),
    }
}

type HashOperation = fn(&Password) -> Result<String, argon2::password_hash::Error>;
type VerifyOperation = fn(&Password, &str) -> Result<(), argon2::password_hash::Error>;

fn hash_operation(password: &Password) -> Result<String, argon2::password_hash::Error> {
    use argon2::{
        PasswordHasher,
        password_hash::{SaltString, rand_core::OsRng},
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
            .map_err(argon2::password_hash::Error::from)?;
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
    };
    #[cfg(not(feature = "cheap-kdf"))]
    let hasher = argon2::Argon2::default();

    hasher
        .hash_password(password.0.as_bytes(), &salt)
        .map(|hash| hash.to_string())
}

fn verify_operation(password: &Password, hash: &str) -> Result<(), argon2::password_hash::Error> {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};

    let parsed = PasswordHash::new(hash)?;
    Argon2::default().verify_password(password.0.as_bytes(), &parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies_a_valid_password() {
        let password: Password = "password123".parse().expect("valid password");
        let hash = hash(&password).expect("hashing succeeds");
        assert!(verify(&password, &hash).expect("verification succeeds"));
    }

    #[test]
    fn production_params_verify_regardless_of_feature() {
        use argon2::{
            Argon2, PasswordHasher,
            password_hash::{SaltString, rand_core::OsRng},
        };

        let password: Password = "password123".parse().expect("valid password");
        let salt = SaltString::generate(&mut OsRng);
        let production_hash = Argon2::default()
            .hash_password(password.as_ref().as_bytes(), &salt)
            .expect("default parameters hash")
            .to_string();

        assert!(production_hash.contains("m=19456"));
        assert!(verify(&password, &production_hash).expect("verification succeeds"));
    }

    #[test]
    fn mismatch_returns_false() {
        let password: Password = "password123".parse().expect("valid password");
        let other: Password = "another-password".parse().expect("valid password");
        let hash = hash(&password).expect("hashing succeeds");

        assert!(!verify(&other, &hash).expect("mismatch is not an error"));
    }

    #[test]
    fn malformed_hash_retains_argon2_source() {
        let password: Password = "password123".parse().expect("valid password");
        let error = verify(&password, "not a valid argon2 hash").expect_err("invalid hash fails");
        let source = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<argon2::password_hash::Error>());

        assert!(source.is_some());
    }

    #[test]
    fn hashing_failure_retains_argon2_source() {
        fn fail(_: &Password) -> Result<String, argon2::password_hash::Error> {
            Err(argon2::password_hash::Error::Algorithm)
        }

        let password: Password = "password123".parse().expect("valid password");
        let error = hash_with(&password, fail).expect_err("injected hash failure");
        let source = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<argon2::password_hash::Error>());

        assert_eq!(source, Some(&argon2::password_hash::Error::Algorithm));
    }

    #[test]
    fn non_password_verification_failure_retains_argon2_source() {
        let password: Password = "password123".parse().expect("valid password");
        let hash =
            "$argon2id$v=1$m=65536,t=2,p=1$c29tZXNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let error = verify(&password, hash).expect_err("unsupported hash version fails");
        let source = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<argon2::password_hash::Error>());

        assert_eq!(source, Some(&argon2::password_hash::Error::Version));
    }
}

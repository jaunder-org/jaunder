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

/// Error returned when a string cannot be parsed as a [`Password`].
#[derive(Debug, Error)]
#[error("password must be at least {MIN_LENGTH} characters")]
pub struct InvalidPassword;

impl FromStr for Password {
    type Err = InvalidPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() < MIN_LENGTH {
            return Err(InvalidPassword);
        }
        Ok(Password(s.to_owned()))
    }
}

impl Password {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Hashes the password using Argon2id with default parameters.
    ///
    /// This is a CPU-intensive operation and should be called from a blocking
    /// context (e.g., via [`tokio::task::spawn_blocking`]).
    pub fn hash(&self) -> Result<String, String> {
        use argon2::{
            password_hash::{rand_core::OsRng, SaltString},
            Argon2, PasswordHasher,
        };

        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(self.0.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| e.to_string())
    }

    /// Verifies the password against a stored Argon2 hash.
    ///
    /// This is a CPU-intensive operation and should be called from a blocking
    /// context (e.g., via [`tokio::task::spawn_blocking`]).
    pub fn verify(&self, hash: &str) -> Result<bool, String> {
        use argon2::{Argon2, PasswordHash, PasswordVerifier};

        let parsed = PasswordHash::new(hash).map_err(|e| e.to_string())?;
        match Argon2::default().verify_password(self.0.as_bytes(), &parsed) {
            Ok(_) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(e) => Err(e.to_string()),
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
        let p: Password = "supersecret".parse().unwrap();
        assert!(!format!("{p:?}").contains("supersecret"));
    }

    #[test]
    fn as_str_returns_original_value() {
        let raw = "correct horse battery staple";
        let p: Password = raw.parse().expect("password meets minimum length");
        assert_eq!(p.as_str(), raw);
    }

    #[test]
    fn hash_and_verify_roundtrip() {
        let raw = "supersecretpassword";
        let p: Password = raw.parse().unwrap();
        let hash = p.hash().expect("hashing should succeed");
        assert!(p.verify(&hash).expect("verification should succeed"));
    }

    #[test]
    fn verify_rejects_wrong_password() {
        let p1: Password = "password123".parse().unwrap();
        let p2: Password = "wrongpassword".parse().unwrap();
        let hash = p1.hash().unwrap();
        assert!(!p2
            .verify(&hash)
            .expect("verification should return false, not error"));
    }

    #[test]
    fn verify_rejects_invalid_hash() {
        let p: Password = "password123".parse().unwrap();
        assert!(p.verify("not a valid argon2 hash").is_err());
    }
}

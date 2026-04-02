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
}

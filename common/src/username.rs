use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A validated username matching `[a-z0-9_-]+`.
///
/// Constructed via [`FromStr`]; invalid strings are rejected at the boundary
/// so interior code works only with already-valid usernames. The
/// `try_from`/`into` serde bridge routes (de)serialization through that same
/// validation, so a `Username` serializes as a plain string and rejects
/// invalid input on the wire — safe to use as a (de)serialized DTO field.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Username(String);

/// Error returned when a string cannot be parsed as a [`Username`].
#[derive(Debug, Error)]
#[error("username must be non-empty and match [a-z0-9_-]+")]
pub struct InvalidUsername;

impl FromStr for Username {
    type Err = InvalidUsername;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        if s.is_empty()
            || !s
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(InvalidUsername);
        }
        Ok(Username(s))
    }
}

impl TryFrom<String> for Username {
    type Error = InvalidUsername;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Username> for String {
    fn from(value: Username) -> Self {
        value.0
    }
}

impl Username {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_parses_valid_names() {
        assert!("alice".parse::<Username>().is_ok());
        assert!("bob-99".parse::<Username>().is_ok());
        assert!("x_y".parse::<Username>().is_ok());
    }

    #[test]
    fn username_normalizes_to_lowercase() {
        let u: Username = "Alice".parse().unwrap();
        assert_eq!(u.as_str(), "alice");

        let u2: Username = "BOB_99".parse().unwrap();
        assert_eq!(u2.as_str(), "bob_99");
    }

    #[test]
    fn username_rejects_invalid_names() {
        assert!("a b".parse::<Username>().is_err());
        assert!("".parse::<Username>().is_err());
        assert!("a@b".parse::<Username>().is_err());
    }

    #[test]
    fn username_display_produces_the_username_string() {
        let u: Username = "alice".parse().unwrap();
        assert_eq!(u.to_string(), "alice");
    }

    #[test]
    fn username_serde_serializes_as_plain_string_and_validates_on_deserialize() {
        let u: Username = "alice".parse().unwrap();
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"alice\"");

        // Deserialize routes through the validating parse (lowercasing too).
        assert_eq!(
            serde_json::from_str::<Username>("\"Alice\"").unwrap(),
            "alice".parse::<Username>().unwrap()
        );

        // Invalid input is rejected at deserialize time.
        assert!(serde_json::from_str::<Username>("\"a b\"").is_err());
    }
}

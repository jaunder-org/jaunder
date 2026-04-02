use std::{fmt, str::FromStr};

use thiserror::Error;

/// A validated username matching `[a-z0-9_-]+`.
///
/// Constructed via [`FromStr`]; invalid strings are rejected at the boundary
/// so interior code works only with already-valid usernames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Username(String);

/// Error returned when a string cannot be parsed as a [`Username`].
#[derive(Debug, Error)]
#[error("username must be non-empty and match [a-z0-9_-]+")]
pub struct InvalidUsername;

impl FromStr for Username {
    type Err = InvalidUsername;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty()
            || !s
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(InvalidUsername);
        }
        Ok(Username(s.to_owned()))
    }
}

impl Username {
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
    fn username_rejects_invalid_names() {
        assert!("Alice".parse::<Username>().is_err());
        assert!("a b".parse::<Username>().is_err());
        assert!("".parse::<Username>().is_err());
        assert!("a@b".parse::<Username>().is_err());
    }
}

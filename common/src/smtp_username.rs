use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty SMTP relay auth username.
///
/// An **identifier, not a secret**, so it adopts the full default [`StrNewtype`]
/// trailer (`Display`, `AsRef<str>`, `Deref<str>`, serde, the validating #438 sqlx
/// bridge, `PartialEq`, owned-`String` conversions). Its sole invariant is
/// non-emptiness — an empty username paired with a set password is a
/// misconfiguration. The paired secret is `SmtpPassword`; making both typed keeps
/// `SmtpCredentials` a fully-typed pair (no same-typed transposition at the lettre
/// `Credentials` boundary). No trim — the stored value is used verbatim, matching
/// `SmtpPassword`. Web-settable wiring (a typed wire arg, non-secret) is #638.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct SmtpUsername(String);

/// Error returned when an SMTP username fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("SMTP username must not be empty")]
pub struct InvalidSmtpUsername;

impl FromStr for SmtpUsername {
    type Err = InvalidSmtpUsername;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidSmtpUsername);
        }
        Ok(SmtpUsername(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty() {
        assert_eq!(
            "user@example.com".parse::<SmtpUsername>().unwrap(),
            "user@example.com"
        );
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<SmtpUsername>().is_err());
        assert_eq!(
            "".parse::<SmtpUsername>().unwrap_err().to_string(),
            "SMTP username must not be empty"
        );
    }

    #[test]
    fn display_and_partial_eq_str() {
        let u: SmtpUsername = "relay-user".parse().unwrap();
        assert_eq!(u.to_string(), "relay-user");
        assert_eq!(u, "relay-user");
    }

    #[test]
    fn serde_round_trips_as_plain_string_and_validates() {
        let u: SmtpUsername = "relay-user".parse().unwrap();
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"relay-user\"");
        assert_eq!(
            serde_json::from_str::<SmtpUsername>("\"relay-user\"").unwrap(),
            "relay-user".parse::<SmtpUsername>().unwrap()
        );
        assert!(serde_json::from_str::<SmtpUsername>("\"\"").is_err());
    }
}

use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

const MIN_LENGTH: usize = 8;
const MAX_LENGTH: usize = 512;

/// Error returned when a submitted password fails the shared input-shape invariant.
#[derive(Debug, Error)]
pub enum InvalidPassword {
    #[error("password must be at least {MIN_LENGTH} characters")]
    PasswordTooShort,
    #[error("password must be at most {MAX_LENGTH} characters")]
    PasswordTooLong,
}

/// Validates the shared shape invariant for submitted plaintext passwords.
///
/// The client-reachable [`ProfferedPassword`] and host-side `host::password::Password`
/// both delegate to this one function, preserving submitted UTF-8 bytes without
/// normalization while counting Unicode scalar values.
///
/// # Errors
///
/// Returns [`InvalidPassword`] when `s` has fewer than eight or more than 512
/// Unicode scalar values.
pub fn validate_password_shape(s: &str) -> Result<(), InvalidPassword> {
    let length = s.chars().count();
    if length < MIN_LENGTH {
        return Err(InvalidPassword::PasswordTooShort);
    }
    if length > MAX_LENGTH {
        return Err(InvalidPassword::PasswordTooLong);
    }
    Ok(())
}

/// A raw plaintext password as **submitted by a client** during registration,
/// login, or password-reset confirmation.
///
/// The serde-capable inbound twin of the host-side secret `Password`, per
/// ADR-0063's inbound-secret variant (`#[str_newtype(secret, serde)]`): redacting
/// `Debug`, `AsRef<str>`, `TryFrom<String>`, and the validating serde bridge —
/// but no `Display`/`Deref`/owned-`String`. It exists only to be validated
/// client-side per ADR-0065, travel client→server, and be converted inward.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, serde)]
pub struct ProfferedPassword(String);

impl FromStr for ProfferedPassword {
    type Err = InvalidPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_password_shape(s)?;
        Ok(Self(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proffered_password_accepts_inclusive_unicode_scalar_bounds() {
        assert_eq!(MIN_LENGTH, 8);
        assert_eq!(MAX_LENGTH, 512);

        let minimum = "é".repeat(MIN_LENGTH);
        let maximum = "a".repeat(MAX_LENGTH);

        assert_eq!(minimum.chars().count(), MIN_LENGTH);
        assert!(minimum.len() > MIN_LENGTH);
        assert!(minimum.parse::<ProfferedPassword>().is_ok());
        assert!(maximum.parse::<ProfferedPassword>().is_ok());
    }

    #[test]
    fn proffered_password_rejects_outside_unicode_scalar_bounds_without_echoing_input() {
        let too_short = "é".repeat(MIN_LENGTH - 1);
        let too_long = "x".repeat(MAX_LENGTH + 1);

        let short = too_short
            .parse::<ProfferedPassword>()
            .expect_err("too short");
        assert!(matches!(short, InvalidPassword::PasswordTooShort));
        assert!(
            short
                .to_string()
                .contains(&format!("at least {MIN_LENGTH} characters"))
        );
        assert!(!short.to_string().contains(&too_short));

        let long = too_long.parse::<ProfferedPassword>().expect_err("too long");
        assert!(matches!(long, InvalidPassword::PasswordTooLong));
        assert!(
            long.to_string()
                .contains(&format!("at most {MAX_LENGTH} characters"))
        );
        assert!(!long.to_string().contains(&too_long));
    }

    #[test]
    fn validation_does_not_normalize_before_counting() {
        let decomposed = "e\u{301}".repeat(7);
        assert_eq!(decomposed.chars().count(), 14);

        let proffered: ProfferedPassword = decomposed.parse().expect("valid scalar count");
        assert_eq!(proffered.as_ref(), decomposed);
    }

    #[test]
    fn proffered_serde_enforces_unicode_scalar_bounds() {
        let accepted: ProfferedPassword = "password123".parse().expect("valid password");
        assert_eq!(
            serde_json::to_string(&accepted).expect("serializes"),
            "\"password123\""
        );
        let roundtrip: ProfferedPassword =
            serde_json::from_str("\"password123\"").expect("deserializes");
        assert_eq!(roundtrip.as_ref(), "password123");

        let too_short = serde_json::to_string(&"é".repeat(MIN_LENGTH - 1)).expect("serializes");
        let too_long = serde_json::to_string(&"a".repeat(MAX_LENGTH + 1)).expect("serializes");
        assert!(serde_json::from_str::<ProfferedPassword>(&too_short).is_err());
        assert!(serde_json::from_str::<ProfferedPassword>(&too_long).is_err());
    }

    #[test]
    fn proffered_debug_is_redacted() {
        let raw = "supersecret123";
        let password: ProfferedPassword = raw.parse().expect("valid password");
        let debug = format!("{password:?}");

        assert!(!debug.contains(raw));
        assert_eq!(debug, "ProfferedPassword([redacted])");
    }
}

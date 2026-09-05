use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// Error returned when an SMTP password fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("SMTP password must not be empty")]
pub struct InvalidSmtpPassword;

/// Validates the shared SMTP relay password shape invariant.
///
/// The client-reachable [`ProfferedSmtpPassword`] and host-only stored secret
/// delegate here, preserving submitted UTF-8 bytes without normalization.
///
/// # Errors
///
/// Returns [`InvalidSmtpPassword`] when `s` is empty.
pub fn validate_smtp_password_shape(s: &str) -> Result<(), InvalidSmtpPassword> {
    if s.is_empty() {
        return Err(InvalidSmtpPassword);
    }
    Ok(())
}

/// Zero-sized proof that a browser SMTP password field has valid shape without
/// retaining its secret value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmtpPasswordShape;

impl FromStr for SmtpPasswordShape {
    type Err = InvalidSmtpPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_smtp_password_shape(s)?;
        Ok(Self)
    }
}

/// A raw SMTP password submitted by a client.
///
/// This inbound twin may travel only from the browser to the server boundary,
/// where it is converted immediately into the host-only stored secret.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, serde)]
pub struct ProfferedSmtpPassword(String);

impl FromStr for ProfferedSmtpPassword {
    type Err = InvalidSmtpPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_smtp_password_shape(s)?;
        Ok(Self(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty_without_normalizing_bytes() {
        let raw = " correct horse relay ";
        let password: ProfferedSmtpPassword = raw.parse().unwrap();
        assert_eq!(password.as_ref(), raw);
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<ProfferedSmtpPassword>().is_err());
        assert!("".parse::<SmtpPasswordShape>().is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "s3cr3t-relay-pw";
        let password: ProfferedSmtpPassword = raw.parse().unwrap();
        let output = format!("{password:?}");
        assert!(!output.contains(raw));
        assert_eq!(output, "ProfferedSmtpPassword([redacted])");
    }
}

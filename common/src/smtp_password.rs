use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty SMTP relay password.
///
/// Adopts the [`StrNewtype`] `secret` surface (ADR-0063 §2): a redacting `Debug`
/// and borrowed `AsRef<str>` access for the lettre `Credentials`, with no
/// `Display`, serde, `Deref`, owned-`String`, or `PartialEq` — so the relay
/// credential cannot be rendered, serialised, logged, or value-compared. The
/// `macros` crate is the authoritative list of what `secret` emits.
///
/// It is a **stored secret**: `#[str_newtype(secret, sqlx)]` layers the validating
/// sqlx bridge back on (like `InviteCode`), so the `site_config` value column
/// decodes straight into `SmtpPassword` through [`FromStr`] — an empty or garbage
/// stored value is rejected as a `ColumnDecode` error at the query boundary (#438),
/// rather than being re-parsed by hand.
///
/// SMTP config is server-side only today (CLI / config-KV → storage → mailer), so
/// there is no inbound `ProfferedSmtpPassword` twin. Making it web-settable (#638)
/// will add that twin and share [`validate_smtp_password_shape`].
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct SmtpPassword(String);

/// Error returned when an SMTP password fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("SMTP password must not be empty")]
pub struct InvalidSmtpPassword;

impl FromStr for SmtpPassword {
    type Err = InvalidSmtpPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_smtp_password_shape(s)?;
        Ok(SmtpPassword(s.to_owned()))
    }
}

/// The shared shape invariant for an SMTP relay password: non-empty. Kept as a
/// free fn so the future `ProfferedSmtpPassword` inbound twin (#638) delegates to
/// the same invariant and cannot drift (mirrors
/// `common::password::validate_password_shape`).
fn validate_smtp_password_shape(s: &str) -> Result<(), InvalidSmtpPassword> {
    if s.is_empty() {
        return Err(InvalidSmtpPassword);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty() {
        assert!("s3cr3t".parse::<SmtpPassword>().is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<SmtpPassword>().is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "s3cr3t-relay-pw";
        let p: SmtpPassword = raw.parse().unwrap();
        let out = format!("{p:?}");
        assert!(!out.contains(raw));
        assert_eq!(out, "SmtpPassword([redacted])");
    }

    #[test]
    fn as_ref_returns_original_value() {
        let raw = "correct horse relay";
        let p: SmtpPassword = raw.parse().unwrap();
        assert_eq!(p.as_ref(), raw);
    }
}

use std::str::FromStr;

use common::smtp_password::{self, InvalidSmtpPassword, ProfferedSmtpPassword};
use macros::StrNewtype;

/// A validated, host-only stored SMTP relay password.
///
/// Its secret newtype surface permits only redacting `Debug` and borrowed
/// access at the mailer or `SQLx` boundary; it cannot be serialized or rendered.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct SmtpPassword(String);

impl FromStr for SmtpPassword {
    type Err = InvalidSmtpPassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        smtp_password::validate_smtp_password_shape(s)?;
        Ok(Self(s.to_owned()))
    }
}

impl TryFrom<ProfferedSmtpPassword> for SmtpPassword {
    type Error = InvalidSmtpPassword;

    fn try_from(password: ProfferedSmtpPassword) -> Result<Self, Self::Error> {
        password.as_ref().parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_preserves_exact_bytes() {
        let raw = " correct horse relay ";
        let password =
            SmtpPassword::try_from(raw.parse::<ProfferedSmtpPassword>().unwrap()).unwrap();
        assert_eq!(password.as_ref(), raw);
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "s3cr3t-relay-pw";
        let password: SmtpPassword = raw.parse().unwrap();
        let output = format!("{password:?}");
        assert!(!output.contains(raw));
        assert_eq!(output, "SmtpPassword([redacted])");
    }
}

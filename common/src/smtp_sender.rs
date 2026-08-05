use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

use crate::mailbox::Mailbox;

/// The `From:` address the outbound mailer sends as — an RFC 5322 mailbox in its stored
/// string form, e.g. `Jaunder <noreply@example.com>`.
///
/// A [`StrNewtype`] over the **stored text** whose invariant is
/// [`Mailbox`]-parseability. It is not a `Mailbox` itself because `site_config.value` is
/// TEXT and the value must round-trip verbatim through the store; parsing it here would
/// re-render it (the canonical form is not always the operator's spelling). Holding the
/// string and pinning the invariant gives both: what is stored is what was set, and what
/// is stored is always addressable.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct SmtpSender(String);

/// Error returned when an SMTP sender is not a parseable mailbox. Carries the parser's
/// reason but not the value: the caller (`site_config list`, the CLI's validate-on-set)
/// already knows the value it offered.
#[derive(Debug, Error)]
#[error("SMTP sender must be an email address, optionally with a display name: {0}")]
pub struct InvalidSmtpSender(String);

impl FromStr for SmtpSender {
    type Err = InvalidSmtpSender;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse only to validate; the operator's spelling is what we store.
        s.parse::<Mailbox>()
            .map_err(|e| InvalidSmtpSender(e.to_string()))?;
        Ok(SmtpSender(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_mailbox_with_a_display_name() {
        let sender: SmtpSender = "Jaunder <noreply@example.com>".parse().unwrap();
        assert_eq!(sender.to_string(), "Jaunder <noreply@example.com>");
    }

    #[test]
    fn accepts_a_bare_address() {
        assert_eq!(
            "noreply@example.com".parse::<SmtpSender>().unwrap(),
            "noreply@example.com"
        );
    }

    #[test]
    fn rejects_a_value_that_is_not_an_email_address() {
        let err = "not-a-valid-email".parse::<SmtpSender>().unwrap_err();
        assert!(
            err.to_string().contains("email address"),
            "the error must name the invariant: {err}"
        );
        assert!("".parse::<SmtpSender>().is_err());
    }

    #[test]
    fn serde_round_trips_as_a_plain_string_and_validates() {
        let sender: SmtpSender = "noreply@example.com".parse().unwrap();
        assert_eq!(
            serde_json::to_string(&sender).unwrap(),
            "\"noreply@example.com\""
        );
        assert_eq!(
            serde_json::from_str::<SmtpSender>("\"noreply@example.com\"").unwrap(),
            sender
        );
        assert!(serde_json::from_str::<SmtpSender>("\"nope\"").is_err());
    }
}

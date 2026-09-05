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

/// Error returned when an SMTP sender is not a parseable mailbox.
///
/// Carries **both** the offending value and the parser's reason (#687 A13): a corrupt
/// `smtp.sender` row surfaces as a `ColumnDecode` carrying this message, and the stored
/// text is the only part of it an operator can act on. A sender address is public
/// configuration, never a secret.
#[derive(Debug, Error)]
#[error("SMTP sender {value:?} must be an email address, optionally with a display name: {reason}")]
pub struct InvalidSmtpSender {
    /// The offending value.
    value: String,
    /// The mailbox parser's own rejection reason.
    reason: String,
}
impl InvalidSmtpSender {
    /// Safe client-facing summary that never echoes the submitted value.
    #[must_use]
    pub fn user_message(&self) -> &'static str {
        "invalid SMTP sender"
    }

    /// Stable low-cardinality telemetry classification.
    #[must_use]
    pub fn telemetry_code(&self) -> &'static str {
        "invalid_smtp_sender"
    }
}

/// The fallback `From:` address used when `smtp.sender` is unset.
pub const DEFAULT_SMTP_SENDER: &str = "Jaunder <noreply@localhost>";

impl Default for SmtpSender {
    /// [`DEFAULT_SMTP_SENDER`] — the infallible construction door the SMTP read uses when
    /// no sender is configured. It is a literal, mailbox-parseable address (pinned by
    /// `the_default_is_a_parseable_mailbox`), so it satisfies the invariant.
    fn default() -> Self {
        Self(DEFAULT_SMTP_SENDER.to_owned())
    }
}

impl FromStr for SmtpSender {
    type Err = InvalidSmtpSender;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse only to validate; the operator's spelling is what we store.
        s.parse::<Mailbox>().map_err(|e| InvalidSmtpSender {
            value: s.to_owned(),
            reason: e.to_string(),
        })?;
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
        assert!(
            err.to_string().contains("not-a-valid-email"),
            "the error must echo the offending value: {err}"
        );
        assert!("".parse::<SmtpSender>().is_err());
    }

    #[test]
    fn safe_error_surfaces_do_not_echo_the_rejected_value() {
        let error = "secret-like-invalid-sender"
            .parse::<SmtpSender>()
            .unwrap_err();
        assert_eq!(error.user_message(), "invalid SMTP sender");
        assert_eq!(error.telemetry_code(), "invalid_smtp_sender");
        assert!(!error.user_message().contains("secret-like-invalid-sender"));
        assert!(
            !error
                .telemetry_code()
                .contains("secret-like-invalid-sender")
        );
    }

    #[test]
    fn the_default_is_a_parseable_mailbox() {
        // `Default` constructs without going through `FromStr`, so this is what keeps the
        // literal honest.
        assert_eq!(SmtpSender::default(), DEFAULT_SMTP_SENDER);
        assert_eq!(
            DEFAULT_SMTP_SENDER.parse::<SmtpSender>().unwrap(),
            SmtpSender::default()
        );
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

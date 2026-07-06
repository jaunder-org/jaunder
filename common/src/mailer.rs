//! Transport-neutral mail types: the [`MailSender`] trait, [`EmailMessage`],
//! [`MailError`], and the dependency-free [`NoopMailSender`] (plus the
//! `test_utils::CapturingMailSender` test double).
//!
//! These live in `common` — which is compiled to WebAssembly via `web`/`hydrate`
//! — precisely because they pull in no native-only crates, so the web layer can
//! name `MailSender`/`EmailMessage` in its `#[server]` functions. The concrete
//! senders that need a SMTP stack or filesystem I/O (`LettreMailSender`,
//! `FileMailSender`) would drag `lettre` into the wasm build, so they live in
//! `server::mailer` instead and are injected per-consumer (see ADR-0016).

use async_trait::async_trait;
use thiserror::Error;

/// A transport-neutral representation of an outbound email message.
///
/// `LettreMailSender` converts this to a `lettre::Message` internally.
/// When HTML support is needed, add `body_html: Option<String>` here —
/// the sender will build a `MultiPart::alternative()` when it is `Some`.
#[derive(Debug, Clone)]
pub struct EmailMessage {
    /// Sender address (e.g. `"Jaunder <noreply@example.com>"`).
    /// When `None`, the sender address from `SmtpConfig` is used.
    pub from: Option<email_address::EmailAddress>,
    /// Recipient addresses. Must contain at least one entry.
    pub to: Vec<email_address::EmailAddress>,
    /// The message subject line.
    pub subject: String,
    /// The plain-text body of the message.
    pub body_text: String,
}

// ---------------------------------------------------------------------------
// MailError
// ---------------------------------------------------------------------------

/// Errors that can occur when sending an email.
#[derive(Debug, Error)]
pub enum MailError {
    /// No mailer is configured (e.g. SMTP host not set).
    #[error("mail sender is not configured")]
    NotConfigured,
    /// The underlying transport returned an error.
    ///
    /// Carries the originating error (lettre address/SMTP error, JSON
    /// serialization, or file-capture I/O) as a typed source rather than a
    /// flattened string.
    #[error("failed to send email: {0}")]
    Send(#[source] Box<dyn std::error::Error + Send + Sync>),
}

// ---------------------------------------------------------------------------
// MailSender trait
// ---------------------------------------------------------------------------

/// Abstraction over different email transports.
#[async_trait]
pub trait MailSender: Send + Sync {
    /// Send an email described by `message`.
    async fn send_email(&self, message: &EmailMessage) -> Result<(), MailError>;
}

// ---------------------------------------------------------------------------
// NoopMailSender
// ---------------------------------------------------------------------------

/// A mail sender that always returns [`MailError::NotConfigured`].
///
/// Used as the default when no SMTP configuration is present.
pub struct NoopMailSender;

#[async_trait]
impl MailSender for NoopMailSender {
    async fn send_email(&self, _message: &EmailMessage) -> Result<(), MailError> {
        Err(MailError::NotConfigured)
    }
}

// ---------------------------------------------------------------------------
// CapturingMailSender (test utilities)
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::{EmailMessage, MailError, MailSender};

    /// A mail sender that captures all sent messages for inspection in tests.
    #[derive(Default)]
    pub struct CapturingMailSender {
        sent: Mutex<Vec<EmailMessage>>,
    }

    impl CapturingMailSender {
        /// Create a new, empty `CapturingMailSender`.
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }

        /// Return a clone of all messages sent so far.
        ///
        /// Recovers from a poisoned mutex (the guarded `Vec` is always in a
        /// consistent state), so this never panics.
        pub fn sent(&self) -> Vec<EmailMessage> {
            self.sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    #[async_trait]
    impl MailSender for CapturingMailSender {
        async fn send_email(&self, message: &EmailMessage) -> Result<(), MailError> {
            self.sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(message.clone());
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailer::test_utils::CapturingMailSender;

    fn parse_email(s: &str) -> email_address::EmailAddress {
        s.parse::<email_address::EmailAddress>()
            .expect("valid email")
    }

    fn create_test_message(from: Option<&str>, to: Vec<&str>, subject: &str) -> EmailMessage {
        EmailMessage {
            from: from.map(parse_email),
            to: to.into_iter().map(parse_email).collect(),
            subject: subject.to_string(),
            body_text: "Hello".to_string(),
        }
    }

    #[tokio::test]
    async fn noop_mail_sender_returns_not_configured() {
        let sender = NoopMailSender;
        let msg = create_test_message(None, vec!["alice@example.com"], "Test");
        let result = sender.send_email(&msg).await;
        assert!(
            matches!(result, Err(MailError::NotConfigured)),
            "expected NotConfigured, got {result:?}"
        );
    }

    #[test]
    fn mail_error_send_preserves_typed_source() {
        use std::error::Error;
        // §3.1a: Send carries the originating error as a typed source rather
        // than a flattened string.
        let io = std::io::Error::other("boom");
        let err = MailError::Send(Box::new(io));
        let source = err.source().expect("Send should expose a source");
        assert!(source.downcast_ref::<std::io::Error>().is_some());
    }

    #[tokio::test]
    async fn capturing_mail_sender_stores_messages() {
        let sender = CapturingMailSender::new();
        let msg = create_test_message(
            Some("sender@example.com"),
            vec!["alice@example.com"],
            "Hello",
        );
        sender.send_email(&msg).await.expect("send should succeed");

        let sent = sender.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].subject, "Hello");
        assert_eq!(sent[0].to, vec![parse_email("alice@example.com")]);
    }

    #[test]
    fn email_message_fields_are_accessible() {
        let msg = EmailMessage {
            from: Some(parse_email("sender@example.com")),
            to: vec![
                parse_email("alice@example.com"),
                parse_email("bob@example.com"),
            ],
            subject: "Hello".to_string(),
            body_text: "Hi there!".to_string(),
        };
        assert_eq!(
            msg.from.as_ref().map(email_address::EmailAddress::as_str),
            Some("sender@example.com")
        );
        assert_eq!(
            msg.to,
            vec![
                parse_email("alice@example.com"),
                parse_email("bob@example.com"),
            ]
        );
        assert_eq!(msg.subject, "Hello");
        assert_eq!(msg.body_text, "Hi there!");
    }

    #[test]
    fn email_message_from_defaults_to_none() {
        let msg = create_test_message(None, vec!["alice@example.com"], "Hello");
        assert!(msg.from.is_none());
    }
}

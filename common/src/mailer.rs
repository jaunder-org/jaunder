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
    #[error("failed to send email: {0}")]
    Send(String),
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
        pub fn new() -> Self {
            Self::default()
        }

        /// Return a clone of all messages sent so far.
        pub fn sent(&self) -> Vec<EmailMessage> {
            self.sent.lock().expect("mutex poisoned").clone()
        }
    }

    #[async_trait]
    impl MailSender for CapturingMailSender {
        async fn send_email(&self, message: &EmailMessage) -> Result<(), MailError> {
            self.sent
                .lock()
                .expect("mutex poisoned")
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

    #[tokio::test]
    async fn noop_mail_sender_returns_not_configured() {
        let sender = NoopMailSender;
        let msg = EmailMessage {
            from: None,
            to: vec!["alice@example.com"
                .parse::<email_address::EmailAddress>()
                .unwrap()],
            subject: "Test".to_string(),
            body_text: "Hello".to_string(),
        };
        let result = sender.send_email(&msg).await;
        assert!(
            matches!(result, Err(MailError::NotConfigured)),
            "expected NotConfigured, got {result:?}"
        );
    }

    #[tokio::test]
    async fn capturing_mail_sender_stores_messages() {
        let sender = CapturingMailSender::new();
        let msg = EmailMessage {
            from: Some(
                "sender@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap(),
            ),
            to: vec!["alice@example.com"
                .parse::<email_address::EmailAddress>()
                .unwrap()],
            subject: "Hello".to_string(),
            body_text: "Hi there!".to_string(),
        };
        sender.send_email(&msg).await.expect("send should succeed");

        let sent = sender.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].subject, "Hello");
        assert_eq!(
            sent[0].to,
            vec!["alice@example.com"
                .parse::<email_address::EmailAddress>()
                .unwrap()]
        );
    }

    #[test]
    fn email_message_fields_are_accessible() {
        let msg = EmailMessage {
            from: Some(
                "sender@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap(),
            ),
            to: vec![
                "alice@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap(),
                "bob@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap(),
            ],
            subject: "Hello".to_string(),
            body_text: "Hi there!".to_string(),
        };
        assert_eq!(
            msg.from.as_ref().map(|a| a.as_str()),
            Some("sender@example.com")
        );
        assert_eq!(
            msg.to,
            vec![
                "alice@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap(),
                "bob@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap(),
            ]
        );
        assert_eq!(msg.subject, "Hello");
        assert_eq!(msg.body_text, "Hi there!");
    }

    #[test]
    fn email_message_from_defaults_to_none() {
        let msg = EmailMessage {
            from: None,
            to: vec!["alice@example.com"
                .parse::<email_address::EmailAddress>()
                .unwrap()],
            subject: "Hello".to_string(),
            body_text: "Hi there!".to_string(),
        };
        assert!(msg.from.is_none());
    }
}

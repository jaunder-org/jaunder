/// A transport-neutral representation of an outbound email message.
///
/// `LettreMailSender` converts this to a `lettre::Message` internally.
/// When HTML support is needed, add `body_html: Option<String>` here —
/// the sender will build a `MultiPart::alternative()` when it is `Some`.
#[derive(Debug, Clone)]
pub struct EmailMessage {
    /// Sender address (e.g. `"Jaunder <noreply@example.com>"`).
    /// When `None`, the sender address from `SmtpConfig` is used.
    pub from: Option<String>,
    /// Recipient addresses. Must contain at least one entry.
    pub to: Vec<String>,
    /// The message subject line.
    pub subject: String,
    /// The plain-text body of the message.
    pub body_text: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_message_fields_are_accessible() {
        let msg = EmailMessage {
            from: Some("sender@example.com".to_string()),
            to: vec![
                "alice@example.com".to_string(),
                "bob@example.com".to_string(),
            ],
            subject: "Hello".to_string(),
            body_text: "Hi there!".to_string(),
        };
        assert_eq!(msg.from.as_deref(), Some("sender@example.com"));
        assert_eq!(msg.to, vec!["alice@example.com", "bob@example.com"]);
        assert_eq!(msg.subject, "Hello");
        assert_eq!(msg.body_text, "Hi there!");
    }

    #[test]
    fn email_message_from_defaults_to_none() {
        let msg = EmailMessage {
            from: None,
            to: vec!["alice@example.com".to_string()],
            subject: "Hello".to_string(),
            body_text: "Hi there!".to_string(),
        };
        assert!(msg.from.is_none());
    }
}

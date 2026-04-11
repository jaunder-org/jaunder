use async_trait::async_trait;
use common::{
    mailer::{EmailMessage, MailError, MailSender},
    smtp::{SmtpConfig, SmtpTlsMode},
};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use thiserror::Error;

// ---------------------------------------------------------------------------
// BuildMailerError
// ---------------------------------------------------------------------------

/// Errors that can occur when constructing a [`LettreMailSender`].
#[derive(Debug, Error)]
pub enum BuildMailerError {
    /// Failed to parse the sender address.
    #[error("invalid sender address: {0}")]
    InvalidSender(String),
    /// Failed to build the SMTP transport.
    #[error("failed to build SMTP transport: {0}")]
    Transport(String),
}

// ---------------------------------------------------------------------------
// LettreMailSender
// ---------------------------------------------------------------------------

/// A [`MailSender`] backed by lettre's async SMTP transport.
pub struct LettreMailSender {
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    sender: Mailbox,
}

impl LettreMailSender {
    /// Build a `LettreMailSender` from an [`SmtpConfig`].
    pub fn from_config(config: &SmtpConfig) -> Result<Self, BuildMailerError> {
        let sender: Mailbox =
            config
                .sender
                .to_string()
                .parse()
                .map_err(|e: lettre::address::AddressError| {
                    BuildMailerError::InvalidSender(e.to_string())
                })?;

        let builder = match config.tls_mode {
            SmtpTlsMode::Plain => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
                    .port(config.port)
            }
            SmtpTlsMode::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
                    .map_err(|e| BuildMailerError::Transport(e.to_string()))?
                    .port(config.port)
            }
            SmtpTlsMode::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
                .map_err(|e| BuildMailerError::Transport(e.to_string()))?
                .port(config.port),
        };

        let builder = match (&config.username, &config.password) {
            (Some(username), Some(password)) => {
                builder.credentials(Credentials::new(username.clone(), password.clone()))
            }
            _ => builder,
        };

        Ok(Self {
            mailer: builder.build(),
            sender,
        })
    }
}

#[async_trait]
impl MailSender for LettreMailSender {
    async fn send_email(&self, message: &EmailMessage) -> Result<(), MailError> {
        let from: Mailbox = message
            .from
            .as_ref()
            .map(|a| a.to_string().parse())
            .transpose()
            .map_err(|e: lettre::address::AddressError| MailError::Send(e.to_string()))?
            .unwrap_or_else(|| self.sender.clone());

        let mut builder = Message::builder().from(from);

        for to_addr in &message.to {
            let mailbox: Mailbox = to_addr
                .to_string()
                .parse()
                .map_err(|e: lettre::address::AddressError| MailError::Send(e.to_string()))?;
            builder = builder.to(mailbox);
        }

        let email = builder
            .subject(&message.subject)
            .body(message.body_text.clone())
            .map_err(|e| MailError::Send(e.to_string()))?;

        self.mailer
            .send(email)
            .await
            .map_err(|e| MailError::Send(e.to_string()))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FileMailSender
// ---------------------------------------------------------------------------

/// A [`MailSender`] that appends each outgoing message as a JSON line to a
/// file on disk.  Used when `JAUNDER_MAIL_CAPTURE_FILE` is set in the
/// environment (typically for end-to-end tests).
pub struct FileMailSender {
    path: std::path::PathBuf,
}

impl FileMailSender {
    /// Create a new `FileMailSender` that writes to `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl MailSender for FileMailSender {
    async fn send_email(&self, message: &EmailMessage) -> Result<(), MailError> {
        use std::io::Write;

        let to: Vec<String> = message.to.iter().map(|a| a.to_string()).collect();
        let record = serde_json::json!({
            "to": to,
            "from": message.from.as_ref().map(|a| a.to_string()),
            "subject": message.subject,
            "body_text": message.body_text,
        });
        let mut line =
            serde_json::to_string(&record).map_err(|e| MailError::Send(e.to_string()))?;
        line.push('\n');

        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| MailError::Send(e.to_string()))?;
            file.write_all(line.as_bytes())
                .map_err(|e| MailError::Send(e.to_string()))?;
            Ok::<(), MailError>(())
        })
        .await
        .map_err(|e| MailError::Send(e.to_string()))?
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use common::smtp::{SmtpConfig, SmtpTlsMode};

    use super::*;

    fn base_config(tls_mode: SmtpTlsMode) -> SmtpConfig {
        SmtpConfig {
            host: "mail.example.com".to_owned(),
            port: 587,
            tls_mode,
            username: None,
            password: None,
            sender: "Jaunder <noreply@example.com>"
                .parse()
                .expect("valid email"),
        }
    }

    #[tokio::test]
    async fn from_config_plain_succeeds() {
        assert!(LettreMailSender::from_config(&base_config(SmtpTlsMode::Plain)).is_ok());
    }

    #[tokio::test]
    async fn from_config_starttls_succeeds() {
        assert!(LettreMailSender::from_config(&base_config(SmtpTlsMode::StartTls)).is_ok());
    }

    #[tokio::test]
    async fn from_config_tls_succeeds() {
        assert!(LettreMailSender::from_config(&base_config(SmtpTlsMode::Tls)).is_ok());
    }

    #[tokio::test]
    async fn from_config_with_credentials_succeeds() {
        let config = SmtpConfig {
            username: Some("user@example.com".to_owned()),
            password: Some("s3cr3t".to_owned()),
            ..base_config(SmtpTlsMode::StartTls)
        };
        assert!(LettreMailSender::from_config(&config).is_ok());
    }

    #[tokio::test]
    async fn from_config_with_only_username_no_credentials_applied() {
        // Credentials are only applied when both username AND password are present.
        let config = SmtpConfig {
            username: Some("user@example.com".to_owned()),
            password: None,
            ..base_config(SmtpTlsMode::StartTls)
        };
        assert!(LettreMailSender::from_config(&config).is_ok());
    }

    #[tokio::test]
    async fn file_mail_sender_appends_json_line() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("mail.jsonl");
        let sender = FileMailSender::new(&path);

        let msg = EmailMessage {
            from: None,
            to: vec!["bob@example.com"
                .parse::<email_address::EmailAddress>()
                .unwrap()],
            subject: "Hello".to_string(),
            body_text: "World".to_string(),
        };
        sender.send_email(&msg).await.expect("send");

        let content = std::fs::read_to_string(&path).expect("read");
        let record: serde_json::Value = serde_json::from_str(content.trim()).expect("parse");
        assert_eq!(record["subject"], "Hello");
        assert_eq!(record["body_text"], "World");
        assert_eq!(record["to"][0], "bob@example.com");
        assert!(record["from"].is_null());
    }

    #[tokio::test]
    async fn file_mail_sender_records_from_field_when_set() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("mail.jsonl");
        let sender = FileMailSender::new(&path);

        let msg = EmailMessage {
            from: Some(
                "sender@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap(),
            ),
            to: vec!["bob@example.com"
                .parse::<email_address::EmailAddress>()
                .unwrap()],
            subject: "Test".to_string(),
            body_text: String::new(),
        };
        sender.send_email(&msg).await.expect("send");

        let content = std::fs::read_to_string(&path).expect("read");
        let record: serde_json::Value = serde_json::from_str(content.trim()).expect("parse");
        assert_eq!(record["from"], "sender@example.com");
    }

    #[tokio::test]
    async fn file_mail_sender_appends_multiple_lines() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("mail.jsonl");
        let sender = FileMailSender::new(&path);

        for i in 0..3u8 {
            let msg = EmailMessage {
                from: None,
                to: vec!["x@example.com"
                    .parse::<email_address::EmailAddress>()
                    .unwrap()],
                subject: format!("msg{i}"),
                body_text: String::new(),
            };
            sender.send_email(&msg).await.expect("send");
        }

        let content = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            let record: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
            assert_eq!(record["subject"], format!("msg{i}"));
        }
    }

    #[tokio::test]
    async fn file_mail_sender_fails_on_directory_path() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let sender = FileMailSender::new(dir.path()); // dir.path() exists as a directory

        let msg = EmailMessage {
            from: None,
            to: vec!["bob@example.com".parse().unwrap()],
            subject: "Hello".to_string(),
            body_text: "World".to_string(),
        };
        let result = sender.send_email(&msg).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed to send email"));
    }
}

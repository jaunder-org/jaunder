//! File-capture mail transport for end-to-end tests.

use async_trait::async_trait;
use common::mailer::{EmailMessage, MailError, MailSender};

/// A [`MailSender`] that appends each outgoing message as a JSON line to a
/// file on disk.  Used for the `mail.jsonl` stream of the `JAUNDER_CAPTURE_DIR`
/// contract (end-to-end tests only); see the `host` crate.
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

        let to: Vec<String> = message.to.iter().map(ToString::to_string).collect();
        let record = serde_json::json!({
            "to": to,
            "from": message.from.as_ref().map(ToString::to_string),
            "subject": message.subject,
            "body_text": message.body_text,
        });
        let Ok(mut line) = serde_json::to_string(&record) else {
            unreachable!("serializing a json! of owned strings is infallible")
        };
        line.push('\n');

        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| MailError::Send(Box::new(e)))?;
            file.write_all(line.as_bytes())
                .map_err(|e| MailError::Send(Box::new(e)))?;
            Ok::<(), MailError>(())
        })
        .await
        // A JoinError only arises if the blocking task panics or is cancelled.
        // This closure only does file I/O (each fallible step handled with `?`,
        // so it returns Err rather than panicking) and is awaited immediately
        // (never cancelled), so no input can drive this arm.
        .map_err(|e| MailError::Send(Box::new(e)))? // cov:ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

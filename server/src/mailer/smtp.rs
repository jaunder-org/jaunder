//! SMTP mail transport backed by [`lettre`].

use async_trait::async_trait;
use common::mailer::{EmailMessage, MailError, MailSender};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use storage::{SmtpConfig, SmtpTlsMode};
use thiserror::Error;

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

/// A [`MailSender`] backed by lettre's async SMTP transport.
pub struct LettreMailSender {
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    sender: Mailbox,
}

impl LettreMailSender {
    /// Build a `LettreMailSender` from an [`SmtpConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error if the sender address is invalid, or if the SMTP
    /// transport cannot be built.
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
                // `builder_dangerous` is lettre's explicit opt-in to an
                // unencrypted connection; Plain mode carries no TLS and is
                // intended only for a trusted local relay.
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
            .map_err(|e: lettre::address::AddressError| MailError::Send(Box::new(e)))?
            .unwrap_or_else(|| self.sender.clone());

        let mut builder = Message::builder().from(from);

        for to_addr in &message.to {
            let mailbox: Mailbox = to_addr
                .to_string()
                .parse()
                .map_err(|e: lettre::address::AddressError| MailError::Send(Box::new(e)))?;
            builder = builder.to(mailbox);
        }

        let Ok(email) = builder
            .subject(&message.subject)
            .body(message.body_text.clone())
        else {
            // lettre's `.body()` only errors when no transfer-encoding fits the
            // bytes; a Rust `String` is guaranteed-valid UTF-8 and always encodes
            // (7bit/8bit/quoted-printable/base64), so this arm is unreachable with
            // our `String` body — no valid input can drive it.
            unreachable!("a String body always encodes; lettre picks a CTE")
        };

        self.mailer
            .send(email)
            .await
            .map_err(|e| MailError::Send(Box::new(e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use common::email::Email;
    use storage::{SmtpConfig, SmtpTlsMode};

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
    async fn send_email_maps_transport_error() {
        // guard:no-backend — no DB
        // Point the mailer at a dead endpoint: nothing listens on 127.0.0.1:0, so
        // the underlying TCP connect fails immediately and `send()` returns an
        // error, exercising the transport-error `map_err` arm.
        let config = SmtpConfig {
            host: "127.0.0.1".to_owned(),
            port: 0,
            tls_mode: SmtpTlsMode::Plain,
            username: None,
            password: None,
            sender: "Jaunder <noreply@example.com>"
                .parse()
                .expect("valid email"),
        };
        let sender = LettreMailSender::from_config(&config).expect("build mailer");

        let msg = EmailMessage {
            from: None,
            to: vec!["bob@example.com".parse().expect("valid email")],
            subject: "Hello".to_owned(),
            body_text: "World".to_owned(),
        };

        let error = sender
            .send_email(&msg)
            .await
            .expect_err("send against a dead endpoint must fail");
        assert!(matches!(error, MailError::Send(_)));
    }

    /// A domain-literal address (`user@[127.0.0.1]`) that `Email` accepts but
    /// lettre's stricter `Mailbox` parser rejects. Used to drive the re-parse
    /// `map_err` arms, which are unreachable through equal parsers.
    fn divergent_address() -> Email {
        "user@[127.0.0.1]"
            .parse()
            .expect("a domain-literal is a valid Email")
    }

    #[tokio::test]
    async fn from_config_rejects_sender_lettre_cannot_parse() {
        // guard:no-backend — no DB
        // `SmtpConfig::sender` (an `email_address::EmailAddress`) accepts the
        // domain-literal, but lettre's Mailbox parser rejects it, so `from_config`
        // maps it to InvalidSender.
        let config = SmtpConfig {
            sender: "user@[127.0.0.1]"
                .parse()
                .expect("email_address accepts a domain-literal"),
            ..base_config(SmtpTlsMode::Plain)
        };
        // `LettreMailSender` is not `Debug`, so match on the result rather than
        // using `expect_err`.
        assert!(matches!(
            LettreMailSender::from_config(&config),
            Err(BuildMailerError::InvalidSender(_))
        ));
    }

    #[tokio::test]
    async fn send_email_rejects_from_lettre_cannot_parse() {
        // guard:no-backend — no DB
        // A valid sender builds the mailer; the message's `from` is the divergent
        // address, so the `from` re-parse fails before any network use.
        let sender =
            LettreMailSender::from_config(&base_config(SmtpTlsMode::Plain)).expect("build mailer");
        let msg = EmailMessage {
            from: Some(divergent_address()),
            to: vec!["bob@example.com".parse().expect("valid email")],
            subject: "Hello".to_owned(),
            body_text: "World".to_owned(),
        };
        let error = sender
            .send_email(&msg)
            .await
            .expect_err("an unparseable from must fail");
        assert!(matches!(error, MailError::Send(_)));
    }

    #[tokio::test]
    async fn send_email_rejects_recipient_lettre_cannot_parse() {
        // guard:no-backend — no DB
        // Valid from (defaulted from config), but a recipient is the divergent
        // address, so the `to` re-parse fails before any network use.
        let sender =
            LettreMailSender::from_config(&base_config(SmtpTlsMode::Plain)).expect("build mailer");
        let msg = EmailMessage {
            from: None,
            to: vec![divergent_address()],
            subject: "Hello".to_owned(),
            body_text: "World".to_owned(),
        };
        let error = sender
            .send_email(&msg)
            .await
            .expect_err("an unparseable recipient must fail");
        assert!(matches!(error, MailError::Send(_)));
    }
}

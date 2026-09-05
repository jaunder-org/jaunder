//! SMTP mail transport backed by [`lettre`].

use async_trait::async_trait;
use common::mailer::{EmailMessage, MailError, MailSender};
use common::smtp_tls_mode::SmtpTlsMode;
use host::smtp_config::SmtpConfig;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::Mailbox,
    transport::smtp::{AsyncSmtpTransportBuilder, authentication::Credentials},
};
use thiserror::Error;

/// Errors that can occur when constructing a [`LettreMailSender`].
#[derive(Debug, Error)]
pub enum BuildMailerError {
    /// The configured sender rendered to a display form lettre rejected.
    ///
    /// The address half is a validated [`Email`](common::email::Email), and
    /// [`common::mailbox::Mailbox`] owns display-name quoting. This is therefore
    /// a defensive error for any remaining mismatch at the lettre boundary.
    #[error("invalid sender address: {0}")]
    InvalidSender(#[source] lettre::address::AddressError),
    /// Failed to build the SMTP transport.
    #[error("failed to build SMTP transport: {0}")]
    Transport(#[source] lettre::transport::smtp::Error),
}

/// An [`EmailMessage`] reached the transport with no recipients.
///
/// [`EmailMessage::to`] is a plain `Vec`, so this is a caller error rather than
/// an impossible state; lettre would otherwise report it as `MissingTo` from
/// deep inside `build()`, naming the symptom rather than the cause.
#[derive(Debug, Error)]
#[error("an email message must have at least one recipient")]
struct NoRecipients;

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
    /// Returns an error if the SMTP transport cannot be built, or if the
    /// configured sender renders to a display form lettre rejects.
    pub fn from_config(config: &SmtpConfig) -> Result<Self, BuildMailerError> {
        Self::from_config_with(
            config,
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay,
            AsyncSmtpTransport::<Tokio1Executor>::relay,
        )
    }

    fn from_config_with(
        config: &SmtpConfig,
        starttls_relay: fn(
            &str,
        )
            -> Result<AsyncSmtpTransportBuilder, lettre::transport::smtp::Error>,
        tls_relay: fn(&str) -> Result<AsyncSmtpTransportBuilder, lettre::transport::smtp::Error>,
    ) -> Result<Self, BuildMailerError> {
        // Fallible, unlike the conversions in `build_message`. Those take an
        // `Email`, which always survives lettre's parser (#297). This takes an
        // operator-authored `SmtpSender`: parse it through `common::Mailbox` to
        // normalize any accepted display-name spelling to Mailbox's RFC-safe render
        // form (#837), then let lettre validate its own boundary.
        let Ok(common_sender) = config
            .sender
            .to_string()
            .parse::<common::mailbox::Mailbox>()
        else {
            unreachable!("SmtpSender invariant guarantees common::Mailbox parseability")
        };
        let sender: Mailbox = common_sender
            .to_string()
            .parse()
            .map_err(BuildMailerError::InvalidSender)?;

        let builder = match config.tls_mode {
            SmtpTlsMode::Plain => {
                // `builder_dangerous` is lettre's explicit opt-in to an
                // unencrypted connection; Plain mode carries no TLS and is
                // intended only for a trusted local relay.
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(config.host.as_ref())
                    .port(config.port.value())
            }
            SmtpTlsMode::StartTls => starttls_relay(config.host.as_ref())
                .map_err(BuildMailerError::Transport)?
                .port(config.port.value()),
            SmtpTlsMode::Tls => tls_relay(config.host.as_ref())
                .map_err(BuildMailerError::Transport)?
                .port(config.port.value()),
        };

        let builder = match (&config.username, &config.password) {
            (Some(username), Some(password)) => {
                // Borrow each credential via `AsRef<str>` and own it for lettre's
                // `Credentials` (the sole plaintext read of the secret password).
                builder.credentials(Credentials::new(
                    username.as_ref().to_owned(),
                    password.as_ref().to_owned(),
                ))
            }
            _ => builder,
        };

        Ok(Self {
            mailer: builder.build(),
            sender,
        })
    }

    /// Build the lettre [`Message`] for an [`EmailMessage`].
    ///
    /// Split out of `send_email` so the address conversion can be asserted
    /// without a live SMTP server — the Nix check derivations are
    /// network-sandboxed, so a test that goes through `send_email` can only
    /// ever observe a transport failure (#297).
    fn build_message(&self, message: &EmailMessage) -> Result<Message, MailError> {
        // Checked here rather than left to lettre: `to` is a plain `Vec`, so an
        // empty one is a caller error, and `build()` would report it as
        // `MissingTo` from the far side of the message builder.
        if message.to.is_empty() {
            return Err(MailError::Send(Box::new(NoRecipients)));
        }

        // Every `Email` survives lettre's display-form parser (see
        // `every_email_survives_lettres_display_form_parser`), so neither of
        // these conversions can fail (#297).
        let from: Mailbox = match message.from.as_ref() {
            Some(addr) => {
                let Ok(mailbox) = addr.to_string().parse::<Mailbox>() else {
                    unreachable!("an Email always parses as a lettre Mailbox")
                };
                mailbox
            }
            None => self.sender.clone(),
        };

        let mut builder = Message::builder().from(from);

        for to_addr in &message.to {
            let Ok(mailbox) = to_addr.to_string().parse::<Mailbox>() else {
                unreachable!("an Email always parses as a lettre Mailbox")
            };
            builder = builder.to(mailbox);
        }

        let Ok(email) = builder
            .subject(&message.subject)
            .body(message.body_text.clone())
        else {
            // `.body()` fails for three reasons: no transfer-encoding fits the
            // bytes, `MissingFrom`, or `MissingTo`. None can happen here. The
            // body is a Rust `String` — guaranteed-valid UTF-8, so it always
            // encodes. `from` is always set just above, and survives the header
            // round-trip lettre performs in `build()` (the guard again). `to` is
            // non-empty, checked at the top of this function.
            unreachable!("from is set, to is non-empty, and a String body always encodes")
        };

        Ok(email)
    }
}

#[async_trait]
impl MailSender for LettreMailSender {
    async fn send_email(&self, message: &EmailMessage) -> Result<(), MailError> {
        let email = self.build_message(message)?;

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
    use common::smtp_port::SmtpPort;
    use common::smtp_tls_mode::SmtpTlsMode;
    use common::test_support::parse_smtp_username;
    use host::smtp_config::SmtpConfig;
    use host::test_support::parse_smtp_password;
    use std::str::FromStr;

    use super::*;

    /// Addresses spanning both grammars: ordinary forms, the two families the
    /// RFC 5322 *display-form* parser chokes on (domain-literals and quoted
    /// local parts), and an internationalized address.
    const ADDRESS_CORPUS: &[&str] = &[
        "user@example.com",
        "USER@Example.COM",
        "user+tag@example.com",
        "first.last@sub.example.co.uk",
        "!#$%&'*+-/=?^_`{|}~@example.com",
        // Domain-literals — RFC 5321 `address-literal`.
        "user@[127.0.0.1]",
        "user@[192.0.2.1]",
        "user@[IPv6:2001:db8::1]",
        // Quoted local parts — RFC 5321 `Quoted-string`.
        "\"quoted\"@example.com",
        "\"has space\"@example.com",
        "\"has@at\"@example.com",
        "\"a<b\"@example.com",
        "\"a,b\"@example.com",
        // Internationalized (EAI). Sending one additionally needs the server to
        // advertise SMTPUTF8, which is a capability question, not a parsing one.
        "user@İ.com",
        "user@münchen.de",
    ];

    /// The invariant the `unreachable!`s in this module rest on: every address
    /// `Email` accepts survives lettre's **display-form** parser and yields a
    /// buildable `Message`.
    ///
    /// `Address` alone is not the invariant. `Headers` stores each header as a
    /// string and re-parses it on `get`, so `MessageBuilder::build` puts every
    /// address back through `Mailbox::from_str` — which is where lettre used to
    /// reject legal addresses it had just rendered (#297).
    ///
    /// **This is the tripwire for the `[patch.crates-io]` entry.** The fix is
    /// carried as a patch until it is released upstream; drop the patch and the
    /// build re-resolves to a lettre that compiles perfectly and mis-parses
    /// quietly, so nothing else fails first. This test is the thing that fails.
    ///
    /// Two limits, stated so they are not mistaken for more than they are: the
    /// corpus is a sample, while the `unreachable!`s claim totality; and this
    /// guard lives beside the code it protects, so one careless edit removes
    /// both.
    #[test]
    fn every_email_survives_lettres_display_form_parser() {
        for raw in ADDRESS_CORPUS {
            // Every corpus entry is a valid `Email` by construction — the point
            // of the corpus is what lettre then makes of it.
            let email = raw
                .parse::<Email>()
                .expect("every ADDRESS_CORPUS entry must be a valid Email");

            let parsed = lettre::message::Mailbox::from_str(email.as_ref());
            assert!(
                parsed.is_ok(),
                "Email accepted {raw:?} but lettre's display-form parser \
                 rejected it — has the lettre [patch.crates-io] entry been \
                 dropped? See #297",
            );
            let Ok(mailbox) = parsed else {
                unreachable!("just asserted ok")
            };

            assert!(
                Message::builder()
                    .from(mailbox.clone())
                    .to(mailbox)
                    .body("body".to_owned())
                    .is_ok(),
                "Email accepted {raw:?} and lettre parsed it, but no Message \
                 could be built — the header round-trip has broken. See #297",
            );
        }
    }

    #[test]
    fn build_message_rejects_an_empty_recipient_list() {
        // `EmailMessage::to` is a plain `Vec`, so this is reachable from a
        // caller. Without the explicit check lettre reports it as `MissingTo`
        // from inside `build()`, which the surrounding `unreachable!` would
        // then turn into a panic.
        let sender =
            LettreMailSender::from_config(&base_config(SmtpTlsMode::Plain)).expect("build mailer");
        let error = sender
            .build_message(&message_to(vec![], None))
            .expect_err("a message with no recipients must be rejected");
        assert!(matches!(error, MailError::Send(_)));
    }

    fn base_config(tls_mode: SmtpTlsMode) -> SmtpConfig {
        SmtpConfig {
            host: "mail.example.com".parse().expect("valid host"),
            port: SmtpPort::default(),
            tls_mode,
            username: None,
            password: None,
            sender: "Jaunder <noreply@example.com>"
                .parse()
                .expect("valid email"),
        }
    }

    fn transport_build_error() -> lettre::transport::smtp::Error {
        lettre::transport::smtp::client::TlsParametersBuilder::new("mail.example.com".to_owned())
            .set_min_tls_version(lettre::transport::smtp::client::TlsVersion::Tlsv10)
            .build_rustls()
            .err()
            .expect("rustls rejects TLS 1.0")
    }

    fn fail_transport_build(
        _host: &str,
    ) -> Result<AsyncSmtpTransportBuilder, lettre::transport::smtp::Error> {
        Err(transport_build_error())
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

    fn assert_transport_builder_source(tls_mode: SmtpTlsMode) {
        let error = LettreMailSender::from_config_with(
            &base_config(tls_mode),
            fail_transport_build,
            fail_transport_build,
        )
        .err()
        .expect("injected TLS construction must fail");
        let error = anyhow::Error::new(error);

        assert!(
            error.chain().any(|source| source
                .downcast_ref::<lettre::transport::smtp::Error>()
                .is_some()),
            "concrete lettre transport source must remain in the chain: {error:#}"
        );
    }

    #[test]
    fn from_config_starttls_retains_transport_builder_source() {
        assert_transport_builder_source(SmtpTlsMode::StartTls);
    }

    #[test]
    fn from_config_tls_retains_transport_builder_source() {
        assert_transport_builder_source(SmtpTlsMode::Tls);
    }

    #[tokio::test]
    async fn from_config_with_credentials_succeeds() {
        let config = SmtpConfig {
            username: Some(parse_smtp_username("user@example.com")),
            password: Some(parse_smtp_password("s3cr3t")),
            ..base_config(SmtpTlsMode::StartTls)
        };
        assert!(LettreMailSender::from_config(&config).is_ok());
    }

    #[tokio::test]
    async fn from_config_with_only_username_no_credentials_applied() {
        // Credentials are only applied when both username AND password are present.
        let config = SmtpConfig {
            username: Some(parse_smtp_username("user@example.com")),
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
        // error, exercising the transport-error `map_err` arm. Port 1 rather than 0:
        // `SmtpPort`'s invariant rules out zero, and nothing listens on either.
        let config = SmtpConfig {
            host: "127.0.0.1".parse().expect("valid host"),
            port: "1".parse().expect("valid port"),
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
        let error = anyhow::Error::new(error);
        assert!(
            error.chain().any(|source| source
                .downcast_ref::<lettre::transport::smtp::Error>()
                .is_some()),
            "concrete lettre transport source must remain downcastable: {error:#}"
        );
    }

    /// Addresses that are RFC-legal, that `Email` accepts, and that lettre's
    /// display-form parser rejected before the patch: a domain-literal and two
    /// quoted local parts (one containing the `@` that the naive split trips
    /// over).
    const DIVERGENT: &[&str] = &[
        "user@[192.0.2.1]",
        "\"has space\"@example.com",
        "\"has@at\"@example.com",
    ];

    fn message_to(to: Vec<Email>, from: Option<Email>) -> EmailMessage {
        EmailMessage {
            from,
            to,
            subject: "Hello".to_owned(),
            body_text: "World".to_owned(),
        }
    }

    #[test]
    fn build_message_accepts_recipients_the_display_parser_rejected() {
        // Driving `build_message` rather than `send_email` matters: against a
        // dead endpoint `send_email` errors regardless, so a send-based test
        // would pass for the wrong reason.
        let sender =
            LettreMailSender::from_config(&base_config(SmtpTlsMode::Plain)).expect("build mailer");
        for raw in DIVERGENT {
            let to: Email = raw.parse().expect("a valid Email");
            let built = sender
                .build_message(&message_to(vec![to.clone()], None))
                .unwrap_or_else(|e| panic!("could not build a message to {raw}: {e:?}"));
            let recipients = built.envelope().to();
            assert_eq!(recipients.len(), 1, "{raw}");
            assert_eq!(recipients[0].to_string(), to.to_string(), "{raw}");
        }
    }

    #[test]
    fn build_message_accepts_a_from_the_display_parser_rejected() {
        let sender =
            LettreMailSender::from_config(&base_config(SmtpTlsMode::Plain)).expect("build mailer");
        for raw in DIVERGENT {
            let from: Email = raw.parse().expect("a valid Email");
            let to: Email = "bob@example.com".parse().expect("valid email");
            let built = sender
                .build_message(&message_to(vec![to], Some(from.clone())))
                .unwrap_or_else(|e| panic!("could not build a message from {raw}: {e:?}"));
            assert_eq!(
                built.envelope().from().map(ToString::to_string),
                Some(from.to_string()),
                "{raw}"
            );
        }
    }

    #[test]
    fn from_config_accepts_a_sender_the_display_parser_rejected() {
        // `SmtpConfig`'s sender is a `common::mailbox::Mailbox`, so this covers
        // named forms too — display-name quoting must not disturb the address.
        for raw in [
            "user@[192.0.2.1]",
            "Jaunder <\"has space\"@example.com>",
            "Acme, Inc <noreply@example.com>",
            "Support: Jaunder <noreply@example.com>",
            "Jaunder (Team) <noreply@example.com>",
            "\"Acme, Inc\" <noreply@example.com>",
        ] {
            let config = SmtpConfig {
                sender: raw.parse().expect("Mailbox accepts it"),
                ..base_config(SmtpTlsMode::Plain)
            };
            // `LettreMailSender` is not `Debug`, so match rather than `expect`.
            assert!(
                LettreMailSender::from_config(&config).is_ok(),
                "could not build a mailer for sender {raw}"
            );
        }
    }
}

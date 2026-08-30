use std::sync::Arc;

use common::mailer::{MailSender, NoopMailSender};
use host::smtp_config::SmtpConfig;
use storage::SiteConfigStorage;

use super::{FileMailSender, LettreMailSender};

/// Picks a mailer implementation based on environment and stored SMTP config.
///
/// In e2e tests, a `Some` capture path (resolved from `JAUNDER_CAPTURE_DIR` at the
/// composition root — see the `host` crate) short-circuits to the file-capture
/// SMTP transport. Absent SMTP configuration alone selects the no-op sender;
/// configuration reads and construction failures propagate with their sources.
///
/// # Errors
///
/// Propagates site-config read failures and failures constructing a present
/// SMTP configuration. An absent SMTP configuration is not an error.
///
/// Lives in `server` (not `storage`) because it depends on lettre and
/// file-capture transports — concerns that the storage crate is deliberately
/// kept agnostic of.
#[tracing::instrument(name = "server.mailer.build", skip(site_config))]
pub async fn build_mailer(
    site_config: &dyn SiteConfigStorage,
    mail_capture: Option<std::path::PathBuf>,
) -> anyhow::Result<Arc<dyn MailSender>> {
    build_mailer_with(site_config, mail_capture, LettreMailSender::from_config).await
}

async fn build_mailer_with(
    site_config: &dyn SiteConfigStorage,
    mail_capture: Option<std::path::PathBuf>,
    build_smtp: impl FnOnce(&SmtpConfig) -> Result<LettreMailSender, super::BuildMailerError>,
) -> anyhow::Result<Arc<dyn MailSender>> {
    if let Some(path) = mail_capture {
        return Ok(Arc::new(FileMailSender::new(path)) as Arc<dyn MailSender>);
    }

    let Some(config) = storage::load_smtp_config(site_config).await? else {
        return Ok(Arc::new(NoopMailSender) as Arc<dyn MailSender>);
    };

    Ok(Arc::new(build_smtp(&config)?) as Arc<dyn MailSender>)
}

#[cfg(test)]
mod tests {
    use rstest::*;
    use rstest_reuse::*;

    use super::*;
    use common::smtp_port::SmtpPort;
    use common::smtp_tls_mode::SmtpTlsMode;
    use host::config_key::SiteConfigKey;
    use storage::test_support::{Backend, backends};

    // guard:no-backend — builds a mailer over a mockall SiteConfigStorage whose reads
    // are all absent; no live database backend
    #[tokio::test]
    async fn build_mailer_returns_sender_when_no_smtp_config() {
        // No smtp.host → load_smtp_config returns Ok(None) → NoopMailSender arm
        let mut store = storage::MockSiteConfigStorage::new();
        store.expect_get_smtp_config().returning(|| Ok(None));
        let sender = build_mailer(&store, None)
            .await
            .expect("absent SMTP selects the no-op sender");
        // NoopMailSender always returns NotConfigured; verify send_email is callable
        let msg = common::mailer::EmailMessage {
            from: None,
            to: vec!["x@example.com".parse().unwrap()],
            subject: "Test".to_string(),
            body_text: String::new(),
        };
        assert!(matches!(
            sender.send_email(&msg).await,
            Err(common::mailer::MailError::NotConfigured)
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn build_mailer_returns_sender_when_smtp_config_present(#[case] backend: Backend) {
        // smtp.host set → load_smtp_config returns Ok(Some(cfg)) → LettreMailSender arm
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set(SiteConfigKey::SmtpHost, "localhost")
            .await
            .unwrap();
        build_mailer(store, None)
            .await
            .expect("present valid SMTP builds the transport");
        // Actual SMTP send requires a server.
    }

    fn smtp_config(tls_mode: SmtpTlsMode) -> SmtpConfig {
        SmtpConfig {
            host: "mail.example.com".parse().expect("valid host"),
            port: SmtpPort::default(),
            tls_mode,
            username: None,
            password: None,
            sender: "Jaunder <noreply@example.com>"
                .parse()
                .expect("valid sender"),
        }
    }

    fn transport_build_error() -> lettre::transport::smtp::Error {
        lettre::transport::smtp::client::TlsParametersBuilder::new("mail.example.com".to_owned())
            .set_min_tls_version(lettre::transport::smtp::client::TlsVersion::Tlsv10)
            .build_rustls()
            .err()
            .expect("rustls rejects TLS 1.0")
    }

    // guard:no-backend — mock storage injects the read failure; no live database
    #[tokio::test]
    async fn mailer_source_chain_retains_config_read_error() {
        let mut store = storage::MockSiteConfigStorage::new();
        store.expect_get_smtp_config().return_once(|| {
            Err(sqlx::Error::Io(std::io::Error::other(
                "injected SMTP config read failure",
            )))
        });

        let error = build_mailer(&store, None)
            .await
            .err()
            .expect("config read failure must fail startup");

        assert!(
            error
                .chain()
                .any(|source| source.downcast_ref::<sqlx::Error>().is_some()),
            "typed SQLx source must remain downcastable: {error:#}"
        );
    }

    // guard:no-backend — private constructor operation injects the lettre failure
    #[tokio::test]
    async fn mailer_source_chain_retains_invalid_sender_error() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config(SmtpTlsMode::Plain))));

        let error = build_mailer_with(&store, None, |_| {
            let source = "not a mailbox"
                .parse::<lettre::message::Mailbox>()
                .expect_err("invalid mailbox yields lettre address error");
            Err(super::super::BuildMailerError::InvalidSender(source))
        })
        .await
        .err()
        .expect("invalid sender must fail startup");

        assert!(
            error.chain().any(|source| source
                .downcast_ref::<lettre::address::AddressError>()
                .is_some()),
            "typed address source must remain downcastable: {error:#}"
        );
    }

    // guard:no-backend — private constructor operation injects the lettre failure
    #[tokio::test]
    async fn mailer_source_chain_retains_transport_build_error() {
        let mut store = storage::MockSiteConfigStorage::new();
        store
            .expect_get_smtp_config()
            .return_once(|| Ok(Some(smtp_config(SmtpTlsMode::StartTls))));

        let error = build_mailer_with(&store, None, |_| {
            Err(super::super::BuildMailerError::Transport(
                transport_build_error(),
            ))
        })
        .await
        .err()
        .expect("transport construction failure must fail startup");

        assert!(
            error.chain().any(|source| source
                .downcast_ref::<lettre::transport::smtp::Error>()
                .is_some()),
            "typed lettre transport source must remain downcastable: {error:#}"
        );
    }

    #[fixture]
    fn capture_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // A `Some` capture path selects the file transport and writes to `<dir>/mail.jsonl`.
    // Injected as a value — no env, no lock (spec Decision 5).
    // guard:no-backend — a capture path short-circuits build_mailer before any store
    // read; no live database backend
    #[rstest]
    #[tokio::test]
    async fn build_mailer_selects_file_sender_when_path_given(capture_dir: tempfile::TempDir) {
        let path = capture_dir.path().join("mail.jsonl");
        let store = storage::MockSiteConfigStorage::new();
        let sender = build_mailer(&store, Some(path.clone()))
            .await
            .expect("capture path selects file sender");
        let msg = common::mailer::EmailMessage {
            from: None,
            to: vec!["x@example.com".parse().unwrap()],
            subject: "Test".to_string(),
            body_text: String::new(),
        };
        sender
            .send_email(&msg)
            .await
            .expect("file sender writes the line");
        assert!(
            path.exists(),
            "FileMailSender must write to the injected capture path"
        );
    }
}

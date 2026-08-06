//! Server-side mail sending.
//!
//! The transport-neutral pieces — the [`MailSender`] trait, `EmailMessage`,
//! `MailError`, and the dependency-free `NoopMailSender`/`CapturingMailSender`
//! — live in [`common::mailer`]. `common` is compiled to WebAssembly (as part
//! of the `web` CSR client), so it must stay free of native-only crates like `lettre`;
//! keeping the trait and data types there lets the web layer name the mailer in
//! `#[server]` functions without pulling a SMTP stack into the wasm build.
//!
//! The concrete senders here depend on `lettre` (async SMTP) and filesystem
//! I/O, so they are server-only. Per [ADR-0016](../../../docs/adr/0016-dependency-injection-and-appstate.md)
//! they are constructed at the composition root and injected per-consumer
//! rather than bundled into shared state:
//!
//! - [`LettreMailSender`] — production SMTP transport (in [`smtp`]).
//! - [`FileMailSender`] — JSON-line capture for end-to-end tests (in [`file`]).
//! - [`build_mailer`] — selects one based on environment and stored config.

mod file;
mod smtp;

use std::sync::Arc;

use common::mailer::{MailSender, NoopMailSender};
use storage::{SiteConfigStorage, load_smtp_config};

pub use file::FileMailSender;
pub use smtp::{BuildMailerError, LettreMailSender};

/// Picks a mailer implementation based on environment and stored SMTP config.
///
/// In e2e tests, a `Some` capture path (resolved from `JAUNDER_CAPTURE_DIR` at the
/// composition root — see the `host` crate) short-circuits to the file-capture
/// transport. Otherwise (`None`, the production default) falls back to the configured
/// SMTP transport, or the no-op sender if configuration is absent or invalid.
///
/// Lives in `server` (not `storage`) because it depends on lettre and
/// file-capture transports — concerns that the storage crate is deliberately
/// kept agnostic of.
#[tracing::instrument(name = "server.mailer.build", skip(site_config))]
pub async fn build_mailer(
    site_config: &dyn SiteConfigStorage,
    mail_capture: Option<std::path::PathBuf>,
) -> Arc<dyn MailSender> {
    if let Some(path) = mail_capture {
        return Arc::new(FileMailSender::new(path)) as Arc<dyn MailSender>;
    }
    match load_smtp_config(site_config).await {
        Ok(Some(cfg)) => match LettreMailSender::from_config(&cfg) {
            Ok(sender) => Arc::new(sender) as Arc<dyn MailSender>,
            Err(_) => Arc::new(NoopMailSender) as Arc<dyn MailSender>,
        },
        Ok(None) | Err(_) => Arc::new(NoopMailSender) as Arc<dyn MailSender>,
    }
}

#[cfg(test)]
mod tests {
    use rstest::*;
    use rstest_reuse::*;

    use super::*;
    use storage::SiteConfigKey;
    use storage::test_support::{Backend, backends};

    // guard:no-backend — builds a mailer over a mockall SiteConfigStorage whose reads
    // are all absent; no live database backend
    #[tokio::test]
    async fn build_mailer_returns_sender_when_no_smtp_config() {
        // No smtp.host → load_smtp_config returns Ok(None) → NoopMailSender arm
        let mut store = storage::MockSiteConfigStorage::new();
        store.expect_get_smtp_config().returning(|| Ok(None));
        let sender = build_mailer(&store, None).await;
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
        let _sender = build_mailer(store, None).await;
        // Just verify build_mailer runs without panic; actual SMTP send requires a server.
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
        let sender = build_mailer(&store, Some(path.clone())).await;
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

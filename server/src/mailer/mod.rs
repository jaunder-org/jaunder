//! Server-side mail sending.
//!
//! The transport-neutral pieces — the [`MailSender`] trait, `EmailMessage`,
//! `MailError`, and the dependency-free `NoopMailSender`/`CapturingMailSender`
//! — live in [`common::mailer`]. `common` is compiled to WebAssembly (via
//! `web`/`hydrate`), so it must stay free of native-only crates like `lettre`;
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
use storage::{load_smtp_config, SiteConfigStorage};

pub use file::FileMailSender;
pub use smtp::{BuildMailerError, LettreMailSender};

/// Picks a mailer implementation based on environment and stored SMTP config.
///
/// In e2e tests, `JAUNDER_MAIL_CAPTURE_FILE` short-circuits to the file-capture
/// transport. Otherwise falls back to the configured SMTP transport, or the
/// no-op sender if configuration is absent or invalid.
///
/// Lives in `server` (not `storage`) because it depends on lettre and
/// file-capture transports — concerns that the storage crate is deliberately
/// kept agnostic of.
#[tracing::instrument(name = "server.mailer.build", skip(site_config))]
pub async fn build_mailer(site_config: &dyn SiteConfigStorage) -> Arc<dyn MailSender> {
    if let Ok(path) = std::env::var("JAUNDER_MAIL_CAPTURE_FILE") {
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
    use std::collections::HashMap;

    use async_trait::async_trait;

    use super::*;

    struct MapConfigStore(HashMap<&'static str, &'static str>);

    #[async_trait]
    impl storage::SiteConfigStorage for MapConfigStore {
        async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
            Ok(self.0.get(key).map(std::string::ToString::to_string))
        }

        async fn set(&self, _key: &str, _value: &str) -> sqlx::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn build_mailer_returns_sender_when_no_smtp_config() {
        // No smtp.host → load_smtp_config returns Ok(None) → NoopMailSender arm
        let store = MapConfigStore(HashMap::new());
        store.set("unused", "value").await.unwrap(); // cover MapConfigStore::set
        let sender = build_mailer(&store).await;
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

    #[tokio::test]
    async fn build_mailer_returns_sender_when_smtp_config_present() {
        // smtp.host set → load_smtp_config returns Ok(Some(cfg)) → LettreMailSender arm
        let store = MapConfigStore(HashMap::from([("smtp.host", "localhost")]));
        let _sender = build_mailer(&store).await;
        // Just verify build_mailer runs without panic; actual SMTP send requires a server.
    }
}

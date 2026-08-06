use common::config_key::SiteConfigKey;
use common::smtp_host::SmtpHost;
use common::smtp_password::SmtpPassword;
use common::smtp_port::SmtpPort;
use common::smtp_sender::SmtpSender;
use common::smtp_username::SmtpUsername;
use thiserror::Error;

use crate::SiteConfigStorage;

// The TLS mode now lives in `common` beside the other SMTP value types, where the
// `#[text_enum]` convention is reachable (`storage` depends on neither `strum` nor
// `macros`, and the sqlx bridge is `#[cfg(feature = "sqlx")]` evaluated in the
// *consuming* crate — see #687 D1a). Re-exported so `storage::smtp::SmtpTlsMode` keeps
// resolving for call sites.
pub use common::smtp_tls_mode::{InvalidSmtpTlsMode, SmtpTlsMode};

// ---------------------------------------------------------------------------
// SmtpConfig
// ---------------------------------------------------------------------------

/// Configuration for the outbound SMTP relay.
///
/// Every field is the value's own type, and every one of them arrives decoded through that
/// type's sqlx bridge by
/// [`SiteConfigStorage::get_smtp_config`](crate::SiteConfigStorage::get_smtp_config) — so an
/// `SmtpConfig` in hand is a configuration that parsed, not one that still might not.
#[derive(Clone, Debug)]
pub struct SmtpConfig {
    /// Relay hostname.
    pub host: SmtpHost,
    /// Port number (default: 587).
    pub port: SmtpPort,
    /// TLS mode (default: [`SmtpTlsMode::StartTls`]).
    pub tls_mode: SmtpTlsMode,
    /// Optional SMTP auth username (a validated non-empty identifier).
    pub username: Option<SmtpUsername>,
    /// Optional SMTP auth password (a redacting secret newtype — never rendered
    /// or logged; read once at the mailer `Credentials` boundary).
    pub password: Option<SmtpPassword>,
    /// Sender address (e.g. `"Jaunder <noreply@example.com>"`).
    pub sender: SmtpSender,
}

// ---------------------------------------------------------------------------
// SmtpConfigError
// ---------------------------------------------------------------------------

/// Errors returned when SMTP configuration is present but invalid.
///
/// The per-field `InvalidPort`/`InvalidTlsMode`/`InvalidSender` variants are gone with the
/// hand-rolled parsing that constructed them (#687 D5): the values now decode inside the
/// query, so a bad one arrives as one kind of thing — a `ColumnDecode` naming its key and
/// echoing the stored text. What the variants carried is carried by the message.
#[derive(Debug, Error)]
pub enum SmtpConfigError {
    /// `smtp.username` or `smtp.password` holds an invalid (e.g. empty) value.
    /// Deliberately **valueless**, and kept distinct for exactly that reason: a credential
    /// is never embedded in an error, so it cannot ride out inside [`Self::Read`].
    #[error("smtp.username or smtp.password holds an invalid value")]
    InvalidCredential,
    /// The SMTP block could not be read: a stored value does not parse as its type, or the
    /// read itself failed. The source's message names the key and echoes the offending
    /// value — the property `load_smtp_config_returns_err_for_*` pins.
    #[error("SMTP configuration could not be read: {0}")]
    Read(#[source] sqlx::Error),
}

// ---------------------------------------------------------------------------
// load_smtp_config
// ---------------------------------------------------------------------------

/// Reads SMTP configuration from the site-config store.
///
/// Returns `Ok(None)` when `smtp.host` is absent — the caller should use a
/// no-op mailer. Returns `Err` when `smtp.host` is present but another field
/// holds an invalid value, so callers can surface a precise error message
/// rather than silently treating misconfiguration as "not configured".
///
/// When optional fields are absent, sensible defaults apply:
///
/// - `smtp.port` defaults to `587`.
/// - `smtp.tls_mode` defaults to `"starttls"`.
/// - `smtp.sender` defaults to `"Jaunder <noreply@localhost>"`.
///
/// The whole read is one call to
/// [`SiteConfigStorage::get_smtp_config`](crate::SiteConfigStorage::get_smtp_config): the
/// parsing lives in the value types' sqlx bridges, so this function has no grammar of its
/// own to drift from theirs. All it adds is the classification below.
///
/// # Errors
///
/// Returns `Err(SmtpConfigError)` if the site config cannot be retrieved from storage.
pub async fn load_smtp_config(
    store: &dyn SiteConfigStorage,
) -> Result<Option<SmtpConfig>, SmtpConfigError> {
    store.get_smtp_config().await.map_err(classify)
}

/// Sorts a failed SMTP read into "a credential is bad" (say so, say nothing more) and
/// everything else (report it in full).
///
/// The split is a disclosure boundary, not a taxonomy: [`SmtpConfigError::Read`] renders
/// its source, and the source of a credential decode failure is the one message in this
/// family that could be built from a secret. `read_value` labels the `ColumnDecode` with
/// the key, which is what makes the two separable here.
fn classify(error: sqlx::Error) -> SmtpConfigError {
    match &error {
        sqlx::Error::ColumnDecode { index, .. }
            if index == SiteConfigKey::SmtpUsername.as_ref()
                || index == SiteConfigKey::SmtpPassword.as_ref() =>
        {
            SmtpConfigError::InvalidCredential
        }
        _ => SmtpConfigError::Read(error),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{backends, Backend};
    use common::test_support::{parse_smtp_password, parse_smtp_username};
    use rstest::*;
    use rstest_reuse::*;

    // -- SmtpTlsMode parsing tests --

    #[test]
    fn tls_mode_parses_plain() {
        assert_eq!("plain".parse::<SmtpTlsMode>().unwrap(), SmtpTlsMode::Plain);
    }

    #[test]
    fn tls_mode_parses_starttls() {
        assert_eq!(
            "starttls".parse::<SmtpTlsMode>().unwrap(),
            SmtpTlsMode::StartTls
        );
    }

    #[test]
    fn tls_mode_parses_tls() {
        assert_eq!("tls".parse::<SmtpTlsMode>().unwrap(), SmtpTlsMode::Tls);
    }

    #[test]
    fn tls_mode_rejects_unknown_string() {
        assert!("ssl".parse::<SmtpTlsMode>().is_err());
        assert!("".parse::<SmtpTlsMode>().is_err());
        assert!("TLS".parse::<SmtpTlsMode>().is_err());
    }

    #[test]
    fn tls_mode_display_renders_expected_strings() {
        assert_eq!(SmtpTlsMode::Plain.to_string(), "plain");
        assert_eq!(SmtpTlsMode::StartTls.to_string(), "starttls");
        assert_eq!(SmtpTlsMode::Tls.to_string(), "tls");
    }

    // -- load_smtp_config tests --

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_returns_none_when_host_absent(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        assert!(load_smtp_config(store).await.unwrap().is_none());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_returns_some_with_all_keys_present(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        for (key, value) in [
            (SiteConfigKey::SmtpHost, "mail.example.com"),
            (SiteConfigKey::SmtpPort, "465"),
            (SiteConfigKey::SmtpTlsMode, "tls"),
            (SiteConfigKey::SmtpUsername, "user@example.com"),
            (SiteConfigKey::SmtpPassword, "s3cr3t"),
            (SiteConfigKey::SmtpSender, "Jaunder <noreply@example.com>"),
        ] {
            store.set(key, value).await.unwrap();
        }

        let config = load_smtp_config(store)
            .await
            .unwrap()
            .expect("expected Some");

        assert_eq!(config.host, "mail.example.com");
        assert_eq!(config.port.value(), 465);
        assert_eq!(config.tls_mode, SmtpTlsMode::Tls);
        assert_eq!(
            config.username,
            Some(parse_smtp_username("user@example.com"))
        );
        assert_eq!(
            config.password.expect("password present").as_ref(),
            "s3cr3t"
        );
        assert_eq!(config.sender, "Jaunder <noreply@example.com>");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_uses_defaults_for_missing_optional_fields(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set(SiteConfigKey::SmtpHost, "relay.example.com")
            .await
            .unwrap();

        let config = load_smtp_config(store)
            .await
            .unwrap()
            .expect("expected Some");

        assert_eq!(config.host, "relay.example.com");
        assert_eq!(config.port, SmtpPort::default());
        assert_eq!(config.tls_mode, SmtpTlsMode::StartTls);
        assert_eq!(config.username, None);
        assert!(config.password.is_none());
        assert_eq!(config.sender, "Jaunder <noreply@localhost>");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_sender(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        store
            .set(SiteConfigKey::SmtpSender, "not-a-valid-email")
            .await
            .unwrap();

        let err = load_smtp_config(store).await.unwrap_err();
        // Asserts the offending value reaches the *message*, deliberately not the error's
        // variant (#687). There is no longer a variant to name: parsing moved into the
        // sqlx bridges, so a bad value arrives as a `ColumnDecode` and
        // `SmtpConfigError::InvalidSender` is gone. The value echo is the property worth
        // protecting — a `matches!` assertion here would have pinned the implementation
        // and blocked that change.
        assert!(
            err.to_string().contains("not-a-valid-email"),
            "the error must echo the offending value; got: {err}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_port(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        store
            .set(SiteConfigKey::SmtpPort, "not-a-port")
            .await
            .unwrap();

        let err = load_smtp_config(store).await.unwrap_err();
        // Message, not variant — see the note on `..._invalid_sender`.
        assert!(
            err.to_string().contains("not-a-port"),
            "the error must echo the offending value; got: {err}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_tls_mode(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        store.set(SiteConfigKey::SmtpTlsMode, "ssl").await.unwrap();

        let err = load_smtp_config(store).await.unwrap_err();
        // Message, not variant — see the note on `..._invalid_sender`.
        assert!(
            err.to_string().contains("ssl"),
            "the error must echo the offending value; got: {err}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_empty_password(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        store.set(SiteConfigKey::SmtpPassword, "").await.unwrap();

        let err = load_smtp_config(store).await.unwrap_err();
        assert!(matches!(err, SmtpConfigError::InvalidCredential));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_empty_username(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set(SiteConfigKey::SmtpHost, "mail.example.com")
            .await
            .unwrap();
        store.set(SiteConfigKey::SmtpUsername, "").await.unwrap();

        let err = load_smtp_config(store).await.unwrap_err();
        assert!(matches!(err, SmtpConfigError::InvalidCredential));
    }

    #[test]
    fn smtp_config_debug_redacts_password() {
        let config = SmtpConfig {
            host: "mail.example.com".parse::<SmtpHost>().unwrap(),
            port: SmtpPort::default(),
            tls_mode: SmtpTlsMode::StartTls,
            username: Some(parse_smtp_username("user@example.com")),
            password: Some(parse_smtp_password("s3cr3t")),
            sender: SmtpSender::default(),
        };

        let out = format!("{config:?}");
        assert!(out.contains("SmtpPassword([redacted])"));
        assert!(!out.contains("s3cr3t"));

        // A cloned config also redacts — this exercises the mandatory
        // `SmtpPassword::clone` derive so it isn't an uncovered llvm-cov region.
        let cloned = format!("{:?}", config.clone());
        assert!(cloned.contains("SmtpPassword([redacted])"));
        assert!(!cloned.contains("s3cr3t"));
    }
}

use common::config_key::SiteConfigKey;
use common::mailbox::Mailbox;
use common::smtp_password::SmtpPassword;
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
#[derive(Clone, Debug)]
pub struct SmtpConfig {
    /// Relay hostname.
    pub host: String,
    /// Port number (default: 587).
    pub port: u16,
    /// TLS mode (default: [`SmtpTlsMode::StartTls`]).
    pub tls_mode: SmtpTlsMode,
    /// Optional SMTP auth username (a validated non-empty identifier).
    pub username: Option<SmtpUsername>,
    /// Optional SMTP auth password (a redacting secret newtype — never rendered
    /// or logged; read once at the mailer `Credentials` boundary).
    pub password: Option<SmtpPassword>,
    /// Sender address (e.g. `"Jaunder <noreply@example.com>"`).
    pub sender: Mailbox,
}

/// The optional SMTP auth credentials, read together as a typed pair by
/// [`SiteConfigStorage::get_smtp_credentials`](crate::SiteConfigStorage::get_smtp_credentials).
///
/// Both fields decode from the `site_config` value column through their validating
/// sqlx bridges, so an empty/garbage stored value is rejected at the query boundary
/// rather than reaching here. `username` is a validated identifier ([`SmtpUsername`],
/// non-secret); `password` is the secret [`SmtpPassword`], whose redacting `Debug`
/// the derived `Debug` inherits.
#[derive(Clone, Debug)]
pub struct SmtpCredentials {
    /// Optional SMTP auth username.
    pub username: Option<SmtpUsername>,
    /// Optional SMTP auth password.
    pub password: Option<SmtpPassword>,
}

// ---------------------------------------------------------------------------
// SmtpConfigError
// ---------------------------------------------------------------------------

/// Errors returned when SMTP configuration is present but invalid.
#[derive(Debug, Error)]
pub enum SmtpConfigError {
    /// `smtp.port` is set to a value that is not a valid port number.
    #[error("smtp.port {0:?} is not a valid port number")]
    InvalidPort(String),
    /// `smtp.tls_mode` is set to an unrecognised value.
    #[error("smtp.tls_mode {0:?} is not valid; expected \"plain\", \"starttls\", or \"tls\"")]
    InvalidTlsMode(String),
    /// `smtp.sender` is set to a value that cannot be parsed as an email address.
    #[error("smtp.sender {0:?} is not a valid email address")]
    InvalidSender(String),
    /// `smtp.username` or `smtp.password` holds an invalid (e.g. empty) value.
    /// Deliberately **valueless** — a credential is never embedded in an error
    /// (unlike the sibling variants, which echo the offending string).
    #[error("smtp.username or smtp.password holds an invalid value")]
    InvalidCredential,
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
/// # Errors
///
/// Returns `Err(SmtpConfigError)` if the site config cannot be retrieved from storage.
pub async fn load_smtp_config(
    store: &dyn SiteConfigStorage,
) -> Result<Option<SmtpConfig>, SmtpConfigError> {
    let Some(host) = store.get(SiteConfigKey::SmtpHost).await.ok().flatten() else {
        return Ok(None);
    };

    let port = match store.get(SiteConfigKey::SmtpPort).await.ok().flatten() {
        None => 587,
        Some(v) => v
            .parse::<u16>()
            .map_err(|_| SmtpConfigError::InvalidPort(v))?,
    };

    let tls_mode = match store.get(SiteConfigKey::SmtpTlsMode).await.ok().flatten() {
        None => SmtpTlsMode::StartTls,
        Some(v) => v
            .parse::<SmtpTlsMode>()
            .map_err(|_| SmtpConfigError::InvalidTlsMode(v))?,
    };

    // Username + password are read together as a typed pair; both decode through
    // their sqlx bridges, so an empty/garbage stored value surfaces as a decode
    // error here (rejected, per the non-empty invariant). `smtp.host` was already
    // read above, so a non-decode storage error can't realistically reach this
    // point; either way the caller (`build_mailer`) maps an `Err` to the safe no-op
    // mailer, so folding both into `InvalidCredential` is sound.
    let SmtpCredentials { username, password } = store
        .get_smtp_credentials()
        .await
        .map_err(|_| SmtpConfigError::InvalidCredential)?;

    let sender_str = store
        .get(SiteConfigKey::SmtpSender)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "Jaunder <noreply@localhost>".to_owned());

    let sender = sender_str
        .parse::<Mailbox>()
        .map_err(|_| SmtpConfigError::InvalidSender(sender_str))?;

    Ok(Some(SmtpConfig {
        host,
        port,
        tls_mode,
        username,
        password,
        sender,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::InMemorySiteConfig;
    use common::test_support::{parse_smtp_password, parse_smtp_username};

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

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_returns_none_when_host_absent() {
        let store = InMemorySiteConfig::new();
        assert!(load_smtp_config(&store).await.unwrap().is_none());
    }

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_returns_some_with_all_keys_present() {
        let store = InMemorySiteConfig::from_pairs([
            ("smtp.host", "mail.example.com"),
            ("smtp.port", "465"),
            ("smtp.tls_mode", "tls"),
            ("smtp.username", "user@example.com"),
            ("smtp.password", "s3cr3t"),
            ("smtp.sender", "Jaunder <noreply@example.com>"),
        ]);

        let config = load_smtp_config(&store)
            .await
            .unwrap()
            .expect("expected Some");

        assert_eq!(config.host, "mail.example.com");
        assert_eq!(config.port, 465);
        assert_eq!(config.tls_mode, SmtpTlsMode::Tls);
        assert_eq!(
            config.username,
            Some(parse_smtp_username("user@example.com"))
        );
        assert_eq!(
            config.password.expect("password present").as_ref(),
            "s3cr3t"
        );
        assert_eq!(
            config.sender,
            "Jaunder <noreply@example.com>".parse::<Mailbox>().unwrap()
        );
    }

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_uses_defaults_for_missing_optional_fields() {
        let store = InMemorySiteConfig::from_pairs([("smtp.host", "relay.example.com")]);

        let config = load_smtp_config(&store)
            .await
            .unwrap()
            .expect("expected Some");

        assert_eq!(config.host, "relay.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.tls_mode, SmtpTlsMode::StartTls);
        assert_eq!(config.username, None);
        assert!(config.password.is_none());
        assert_eq!(
            config.sender,
            "Jaunder <noreply@localhost>".parse::<Mailbox>().unwrap()
        );
    }

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_sender() {
        let store = InMemorySiteConfig::from_pairs([
            ("smtp.host", "mail.example.com"),
            ("smtp.sender", "not-a-valid-email"),
        ]);

        let err = load_smtp_config(&store).await.unwrap_err();
        // Asserts the offending value reaches the *message*, deliberately not the error's
        // variant (#687). The variant is about to stop existing: once parsing moves into
        // the sqlx bridges, a bad value arrives as a `ColumnDecode` and
        // `SmtpConfigError::InvalidSender` becomes unconstructible. The value echo is the
        // property worth protecting — a `matches!` assertion here would instead pin the
        // implementation and block that change.
        assert!(
            err.to_string().contains("not-a-valid-email"),
            "the error must echo the offending value; got: {err}"
        );
    }

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_port() {
        let store = InMemorySiteConfig::from_pairs([
            ("smtp.host", "mail.example.com"),
            ("smtp.port", "not-a-port"),
        ]);

        let err = load_smtp_config(&store).await.unwrap_err();
        // Message, not variant — see the note on `..._invalid_sender`.
        assert!(
            err.to_string().contains("not-a-port"),
            "the error must echo the offending value; got: {err}"
        );
    }

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_tls_mode() {
        let store = InMemorySiteConfig::from_pairs([
            ("smtp.host", "mail.example.com"),
            ("smtp.tls_mode", "ssl"),
        ]);

        let err = load_smtp_config(&store).await.unwrap_err();
        // Message, not variant — see the note on `..._invalid_sender`.
        assert!(
            err.to_string().contains("ssl"),
            "the error must echo the offending value; got: {err}"
        );
    }

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_empty_password() {
        let store = InMemorySiteConfig::from_pairs([
            ("smtp.host", "mail.example.com"),
            ("smtp.password", ""),
        ]);

        let err = load_smtp_config(&store).await.unwrap_err();
        assert!(matches!(err, SmtpConfigError::InvalidCredential));
    }

    // guard:no-backend — reads SMTP config from an injected mock SiteConfigStorage; no live database backend
    #[tokio::test]
    async fn load_smtp_config_returns_err_for_empty_username() {
        let store = InMemorySiteConfig::from_pairs([
            ("smtp.host", "mail.example.com"),
            ("smtp.username", ""),
        ]);

        let err = load_smtp_config(&store).await.unwrap_err();
        assert!(matches!(err, SmtpConfigError::InvalidCredential));
    }

    #[test]
    fn smtp_config_debug_redacts_password() {
        let config = SmtpConfig {
            host: "mail.example.com".to_owned(),
            port: 587,
            tls_mode: SmtpTlsMode::StartTls,
            username: Some(parse_smtp_username("user@example.com")),
            password: Some(parse_smtp_password("s3cr3t")),
            sender: "Jaunder <noreply@example.com>".parse::<Mailbox>().unwrap(),
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

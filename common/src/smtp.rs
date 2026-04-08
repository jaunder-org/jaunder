use std::{fmt, str::FromStr};

use thiserror::Error;

use crate::storage::SiteConfigStorage;

// ---------------------------------------------------------------------------
// SmtpTlsMode
// ---------------------------------------------------------------------------

/// The TLS mode to use when connecting to the SMTP relay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmtpTlsMode {
    /// Unencrypted plain SMTP connection.
    Plain,
    /// Upgrade to TLS using STARTTLS after connecting.
    StartTls,
    /// Connect using TLS from the start (implicit TLS / SMTPS).
    Tls,
}

impl fmt::Display for SmtpTlsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmtpTlsMode::Plain => write!(f, "plain"),
            SmtpTlsMode::StartTls => write!(f, "starttls"),
            SmtpTlsMode::Tls => write!(f, "tls"),
        }
    }
}

/// Error returned when a string does not name a valid [`SmtpTlsMode`].
#[derive(Debug, Error)]
#[error("invalid SMTP TLS mode: {0:?}")]
pub struct InvalidSmtpTlsMode(String);

impl FromStr for SmtpTlsMode {
    type Err = InvalidSmtpTlsMode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "plain" => Ok(SmtpTlsMode::Plain),
            "starttls" => Ok(SmtpTlsMode::StartTls),
            "tls" => Ok(SmtpTlsMode::Tls),
            other => Err(InvalidSmtpTlsMode(other.to_owned())),
        }
    }
}

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
    /// Optional SMTP auth username.
    pub username: Option<String>,
    /// Optional SMTP auth password.
    pub password: Option<String>,
    /// Sender address (e.g. `"Jaunder <noreply@example.com>"`).
    pub sender: email_address::EmailAddress,
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
pub async fn load_smtp_config(
    store: &dyn SiteConfigStorage,
) -> Result<Option<SmtpConfig>, SmtpConfigError> {
    let Some(host) = store.get("smtp.host").await.ok().flatten() else {
        return Ok(None);
    };

    let port = match store.get("smtp.port").await.ok().flatten() {
        None => 587,
        Some(v) => v
            .parse::<u16>()
            .map_err(|_| SmtpConfigError::InvalidPort(v))?,
    };

    let tls_mode = match store.get("smtp.tls_mode").await.ok().flatten() {
        None => SmtpTlsMode::StartTls,
        Some(v) => v
            .parse::<SmtpTlsMode>()
            .map_err(|_| SmtpConfigError::InvalidTlsMode(v))?,
    };

    let username = store.get("smtp.username").await.ok().flatten();
    let password = store.get("smtp.password").await.ok().flatten();

    let sender_str = store
        .get("smtp.sender")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "Jaunder <noreply@localhost>".to_owned());

    let sender = sender_str
        .parse::<email_address::EmailAddress>()
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
    use std::collections::HashMap;

    use async_trait::async_trait;

    use super::*;

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

    /// Minimal in-memory implementation of [`SiteConfigStorage`] for tests.
    struct MapConfigStore(HashMap<&'static str, &'static str>);

    #[async_trait]
    impl SiteConfigStorage for MapConfigStore {
        async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
            Ok(self.0.get(key).map(|v| v.to_string()))
        }

        async fn set(&self, _key: &str, _value: &str) -> sqlx::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn map_config_store_set_returns_ok() {
        let store = MapConfigStore(HashMap::new());
        store.set("smtp.host", "mail.example.com").await.unwrap();
    }

    #[tokio::test]
    async fn load_smtp_config_returns_none_when_host_absent() {
        let store = MapConfigStore(HashMap::new());
        assert!(load_smtp_config(&store).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn load_smtp_config_returns_some_with_all_keys_present() {
        let store = MapConfigStore(HashMap::from([
            ("smtp.host", "mail.example.com"),
            ("smtp.port", "465"),
            ("smtp.tls_mode", "tls"),
            ("smtp.username", "user@example.com"),
            ("smtp.password", "s3cr3t"),
            ("smtp.sender", "Jaunder <noreply@example.com>"),
        ]));

        let config = load_smtp_config(&store)
            .await
            .unwrap()
            .expect("expected Some");

        assert_eq!(config.host, "mail.example.com");
        assert_eq!(config.port, 465);
        assert_eq!(config.tls_mode, SmtpTlsMode::Tls);
        assert_eq!(config.username, Some("user@example.com".to_owned()));
        assert_eq!(config.password, Some("s3cr3t".to_owned()));
        assert_eq!(
            config.sender,
            "Jaunder <noreply@example.com>"
                .parse::<email_address::EmailAddress>()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn load_smtp_config_uses_defaults_for_missing_optional_fields() {
        let store = MapConfigStore(HashMap::from([("smtp.host", "relay.example.com")]));

        let config = load_smtp_config(&store)
            .await
            .unwrap()
            .expect("expected Some");

        assert_eq!(config.host, "relay.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.tls_mode, SmtpTlsMode::StartTls);
        assert_eq!(config.username, None);
        assert_eq!(config.password, None);
        assert_eq!(
            config.sender,
            "Jaunder <noreply@localhost>"
                .parse::<email_address::EmailAddress>()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_sender() {
        let store = MapConfigStore(HashMap::from([
            ("smtp.host", "mail.example.com"),
            ("smtp.sender", "not-a-valid-email"),
        ]));

        let err = load_smtp_config(&store).await.unwrap_err();
        assert!(matches!(err, SmtpConfigError::InvalidSender(_)));
    }

    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_port() {
        let store = MapConfigStore(HashMap::from([
            ("smtp.host", "mail.example.com"),
            ("smtp.port", "not-a-port"),
        ]));

        let err = load_smtp_config(&store).await.unwrap_err();
        assert!(matches!(err, SmtpConfigError::InvalidPort(_)));
    }

    #[tokio::test]
    async fn load_smtp_config_returns_err_for_invalid_tls_mode() {
        let store = MapConfigStore(HashMap::from([
            ("smtp.host", "mail.example.com"),
            ("smtp.tls_mode", "ssl"),
        ]));

        let err = load_smtp_config(&store).await.unwrap_err();
        assert!(matches!(err, SmtpConfigError::InvalidTlsMode(_)));
    }
}

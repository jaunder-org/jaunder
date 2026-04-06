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
// load_smtp_config
// ---------------------------------------------------------------------------

/// Reads SMTP configuration from the site-config store.
///
/// Returns `None` when `smtp.host` is absent — in that case the caller should
/// use a no-op mailer. If `smtp.host` is present but other optional fields are
/// absent or invalid, sensible defaults are used:
///
/// - `smtp.port` defaults to `587`.
/// - `smtp.tls_mode` defaults to `"starttls"`.
/// - `smtp.sender` defaults to `"Jaunder <noreply@localhost>"`.
///
/// Returns `None` if `smtp.sender` is present but cannot be parsed as a valid
/// email address, treating an invalid sender as a misconfigured mailer.
pub async fn load_smtp_config(store: &dyn SiteConfigStorage) -> Option<SmtpConfig> {
    let host = store.get("smtp.host").await.ok().flatten()?;

    let port = store
        .get("smtp.port")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(587);

    let tls_mode = store
        .get("smtp.tls_mode")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<SmtpTlsMode>().ok())
        .unwrap_or(SmtpTlsMode::StartTls);

    let username = store.get("smtp.username").await.ok().flatten();

    let password = store.get("smtp.password").await.ok().flatten();

    let sender_str = store
        .get("smtp.sender")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "Jaunder <noreply@localhost>".to_owned());

    let sender = sender_str.parse::<email_address::EmailAddress>().ok()?;

    Some(SmtpConfig {
        host,
        port,
        tls_mode,
        username,
        password,
        sender,
    })
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

    // -- load_smtp_config tests --

    /// Minimal in-memory implementation of [`SiteConfigStorage`] for tests.
    struct MapConfigStore(HashMap<&'static str, &'static str>);

    #[async_trait]
    impl SiteConfigStorage for MapConfigStore {
        async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
            Ok(self.0.get(key).map(|v| v.to_string()))
        }

        async fn set(&self, _key: &str, _value: &str) -> sqlx::Result<()> {
            unimplemented!("not needed in tests")
        }
    }

    #[tokio::test]
    async fn load_smtp_config_returns_none_when_host_absent() {
        let store = MapConfigStore(HashMap::new());
        assert!(load_smtp_config(&store).await.is_none());
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

        let config = load_smtp_config(&store).await.expect("expected Some");

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

        let config = load_smtp_config(&store).await.expect("expected Some");

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
    async fn load_smtp_config_returns_none_for_invalid_sender() {
        let store = MapConfigStore(HashMap::from([
            ("smtp.host", "mail.example.com"),
            ("smtp.sender", "not-a-valid-email"),
        ]));

        assert!(load_smtp_config(&store).await.is_none());
    }
}

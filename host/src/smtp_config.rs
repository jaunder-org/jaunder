//! Host-side outbound SMTP relay configuration.

use common::smtp_host::SmtpHost;
use common::smtp_password::SmtpPassword;
use common::smtp_port::SmtpPort;
use common::smtp_sender::SmtpSender;
use common::smtp_tls_mode::SmtpTlsMode;
use common::smtp_username::SmtpUsername;

/// Validated configuration for the host's outbound SMTP relay.
///
/// Every field uses its own validated [`common`] value type, so an `SmtpConfig`
/// in hand contains values ready for the host mailer rather than unparsed input.
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

use std::fmt;
use std::str::FromStr;

use macros::SqlxBridge;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// The TCP port of the outbound SMTP relay.
///
/// Holds a `u16` — the port's actual domain — but is **stored as text**, because
/// `site_config.value` is `TEXT NOT NULL` on both backends. That is what
/// `#[sqlx_bridge(text)]` expresses: `Type`/`Encode`/`Decode` all move to `String`, and
/// the decode parses back out through the `FromStr` below, so a non-numeric stored value
/// is a `ColumnDecode` error rather than a silently coerced `0` (the SQLite/Postgres
/// divergence `CAST(value AS INTEGER)` would have introduced — ADR-0019).
///
/// [`SqlxBridge`] emits the bridge and **nothing else** — by charter it never leaks a
/// constructor — so the `FromStr` and `Display` below are hand-written. They are also
/// the only doors in and out, which keeps the stored text and the held number in one
/// grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, SqlxBridge)]
#[sqlx_bridge(text)]
pub struct SmtpPort(u16);

/// Error returned when a string does not name a valid TCP port.
///
/// Carries **both** the offending value and the parser's reason (#687 A13). The value is
/// load-bearing at the decode seam: a bad `smtp.port` row reaches the operator as a
/// `ColumnDecode` whose message is this one, and "invalid digit found in string" without
/// the digits is not an actionable report. A port is a configuration value, never a
/// secret — unlike the host-only SMTP password, whose error stays valueless.
#[derive(Debug, Error)]
#[error("SMTP port {value:?} must be a number in 1..=65535: {reason}")]
pub struct InvalidSmtpPort {
    /// The offending value.
    value: String,
    /// The parser's own rejection reason.
    reason: String,
}
impl InvalidSmtpPort {
    /// Safe client-facing summary that never echoes the submitted value.
    #[must_use]
    pub fn user_message(&self) -> &'static str {
        "invalid SMTP port"
    }

    /// Stable low-cardinality telemetry classification.
    #[must_use]
    pub fn telemetry_code(&self) -> &'static str {
        "invalid_smtp_port"
    }
}

/// The IANA submission port — the default when `smtp.port` is unset.
pub const DEFAULT_SMTP_PORT: u16 = 587;

impl SmtpPort {
    /// The port number, for the mailer's connection builder.
    #[must_use]
    pub fn value(self) -> u16 {
        self.0
    }
}

impl Default for SmtpPort {
    /// [`DEFAULT_SMTP_PORT`] — the infallible construction door the SMTP read uses when
    /// no port is configured. 587 is non-zero, so it satisfies the invariant by
    /// inspection.
    fn default() -> Self {
        Self(DEFAULT_SMTP_PORT)
    }
}

impl FromStr for SmtpPort {
    type Err = InvalidSmtpPort;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let reject = |reason: String| InvalidSmtpPort {
            value: s.to_owned(),
            reason,
        };
        let port = s.parse::<u16>().map_err(|e| reject(e.to_string()))?;
        if port == 0 {
            return Err(reject("a port must not be zero".to_owned()));
        }
        Ok(SmtpPort(port))
    }
}

impl Serialize for SmtpPort {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u16(self.0)
    }
}

impl<'de> Deserialize<'de> for SmtpPort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let port = u16::deserialize(deserializer)?;
        port.to_string().parse().map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for SmtpPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_port_number() {
        assert_eq!("587".parse::<SmtpPort>().unwrap().value(), 587);
        assert_eq!("465".parse::<SmtpPort>().unwrap().value(), 465);
    }

    #[test]
    fn rejects_a_non_numeric_value() {
        let err = "not-a-port".parse::<SmtpPort>().unwrap_err();
        assert!(
            err.to_string().contains("port"),
            "the error must name the invariant: {err}"
        );
        assert!(
            err.to_string().contains("not-a-port"),
            "the error must echo the offending value: {err}"
        );
        assert!("".parse::<SmtpPort>().is_err());
    }

    #[test]
    fn safe_error_surfaces_do_not_echo_the_rejected_value() {
        let error = "secret-like-invalid-port".parse::<SmtpPort>().unwrap_err();
        assert_eq!(error.user_message(), "invalid SMTP port");
        assert_eq!(error.telemetry_code(), "invalid_smtp_port");
        assert!(!error.user_message().contains("secret-like-invalid-port"));
        assert!(!error.telemetry_code().contains("secret-like-invalid-port"));
    }

    #[test]
    fn defaults_to_the_submission_port() {
        assert_eq!(SmtpPort::default().value(), DEFAULT_SMTP_PORT);
        assert_eq!(SmtpPort::default().to_string(), "587");
    }

    #[test]
    fn rejects_a_value_outside_the_port_range() {
        assert!("70000".parse::<SmtpPort>().is_err());
        // Zero is syntactically a `u16` but not a connectable port.
        let err = "0".parse::<SmtpPort>().unwrap_err();
        assert!(err.to_string().contains("zero"), "got: {err}");
    }

    #[test]
    fn display_renders_the_number_back() {
        assert_eq!("465".parse::<SmtpPort>().unwrap().to_string(), "465");
        assert_eq!("587".parse::<SmtpPort>().unwrap().to_string(), "587");
    }
}

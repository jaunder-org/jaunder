use std::fmt;
use std::str::FromStr;

use macros::SqlxBridge;
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

/// Error returned when a string does not name a valid TCP port. Carries the parser's
/// reason rather than the offending value; the caller already has the value.
#[derive(Debug, Error)]
#[error("SMTP port must be a number in 1..=65535: {0}")]
pub struct InvalidSmtpPort(String);

impl SmtpPort {
    /// The port number, for the mailer's connection builder.
    #[must_use]
    pub fn value(self) -> u16 {
        self.0
    }
}

impl FromStr for SmtpPort {
    type Err = InvalidSmtpPort;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let port = s
            .parse::<u16>()
            .map_err(|e| InvalidSmtpPort(e.to_string()))?;
        if port == 0 {
            return Err(InvalidSmtpPort("port must not be zero".to_owned()));
        }
        Ok(SmtpPort(port))
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
        assert!("".parse::<SmtpPort>().is_err());
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

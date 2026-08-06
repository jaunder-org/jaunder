//! The TLS mode used when connecting to the outbound SMTP relay.

/// How the mailer secures its connection to the relay.
///
/// A closed string enum (`#[text_enum]`, ADR-0075 as amended by #746) stored as its token
/// in `site_config.value`, so an unrecognised token is a rejection rather than a
/// plausible-looking guess — which for a *transport security* setting is the difference
/// between failing to send and sending in the clear.
///
/// The tokens are spelled out rather than taken from `serialize_all = "snake_case"`:
/// `StartTls`'s `snake_case` is `start_tls`, but the shipped, operator-facing token is
/// `starttls`.
///
/// It lives here rather than beside the mailer in `storage` for the reasons in ADR-0091
/// / D1a: `storage` depends on neither `strum` nor `macros`, and the bridge is
/// `#[cfg(feature = "sqlx")]` evaluated in the *consuming* crate, so there the attribute
/// would silently emit nothing.
#[macros::text_enum(
    sqlx,
    error = InvalidSmtpTlsMode,
    message = "SMTP TLS mode must be \"plain\", \"starttls\", or \"tls\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SmtpTlsMode {
    /// Unencrypted plain SMTP connection.
    #[strum(serialize = "plain")]
    Plain,
    /// Upgrade to TLS using STARTTLS after connecting.
    #[strum(serialize = "starttls")]
    StartTls,
    /// Connect using TLS from the start (implicit TLS / SMTPS).
    #[strum(serialize = "tls")]
    Tls,
}

impl Default for SmtpTlsMode {
    /// STARTTLS — the mode a relay is most likely to speak, and the one that is encrypted.
    /// The default is here rather than at the read site so "unset means STARTTLS" is
    /// stated once, beside the tokens it is one of.
    ///
    /// Written out rather than `#[derive(Default)]`: the derive would have to sit among
    /// the attribute macro's injected `strum` derives, and `#[default]` on a variant reads
    /// as a token marker beside `#[strum(serialize = …)]` when it is not one.
    fn default() -> Self {
        Self::StartTls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn defaults_to_starttls() {
        assert_eq!(SmtpTlsMode::default(), SmtpTlsMode::StartTls);
    }

    #[test]
    fn every_token_round_trips() {
        for (mode, token) in [
            (SmtpTlsMode::Plain, "plain"),
            (SmtpTlsMode::StartTls, "starttls"),
            (SmtpTlsMode::Tls, "tls"),
        ] {
            assert_eq!(SmtpTlsMode::from_str(token).unwrap(), mode, "for {token}");
            assert_eq!(mode.to_string(), token);
        }
    }

    #[test]
    fn an_unrecognised_token_is_rejected() {
        for bad in ["ssl", "", "TLS", "start_tls"] {
            assert!(SmtpTlsMode::from_str(bad).is_err(), "{bad} must reject");
        }
        let err = SmtpTlsMode::from_str("ssl").unwrap_err();
        assert!(
            err.to_string().contains("plain"),
            "the named error lists the valid tokens: {err}"
        );
    }
}

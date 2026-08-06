use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty SMTP relay hostname.
///
/// Its sole invariant is non-emptiness: `smtp.host` being set is what makes the whole
/// SMTP block live (an unset host means "use the no-op mailer"), so an empty stored host
/// is a misconfiguration rather than a way to say "unset". Deliberately **not** a
/// DNS-name or URL validator — the relay may be a bare label, an IP literal, or a
/// container name, and rejecting those would be a new restriction, not a captured
/// invariant.
///
/// Takes the full default [`StrNewtype`] trailer (`Display`, `AsRef<str>`, `Deref<str>`,
/// serde, the validating #438 sqlx bridge, `PartialEq`, owned-`String` conversions),
/// matching its sibling [`crate::smtp_username::SmtpUsername`] — it is an identifier,
/// not a secret. No trim: the stored value is used verbatim.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct SmtpHost(String);

/// Error returned when an SMTP host fails its shape invariant (empty).
///
/// **Deliberately valueless**, unlike its `SmtpPort`/`SmtpSender` siblings, which carry the
/// offending text (#687). Those types reject many different inputs, so an operator needs to
/// be told *which* one was stored. This type rejects exactly one — the empty string — so a
/// carried value would always be `""` and the message already names it. Value-carrying is
/// there to make a bad row identifiable, not as a uniform shape to satisfy.
#[derive(Debug, Error)]
#[error("SMTP host must not be empty")]
pub struct InvalidSmtpHost;

impl FromStr for SmtpHost {
    type Err = InvalidSmtpHost;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidSmtpHost);
        }
        Ok(SmtpHost(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_non_empty_host() {
        assert_eq!(
            "mail.example.com".parse::<SmtpHost>().unwrap(),
            "mail.example.com"
        );
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<SmtpHost>().is_err());
        assert_eq!(
            "".parse::<SmtpHost>().unwrap_err().to_string(),
            "SMTP host must not be empty"
        );
    }

    #[test]
    fn display_renders_the_host_verbatim() {
        let host: SmtpHost = "relay.example.com".parse().unwrap();
        assert_eq!(host.to_string(), "relay.example.com");
    }

    #[test]
    fn serde_round_trips_as_a_plain_string_and_validates() {
        let host: SmtpHost = "relay.example.com".parse().unwrap();
        assert_eq!(
            serde_json::to_string(&host).unwrap(),
            "\"relay.example.com\""
        );
        assert_eq!(
            serde_json::from_str::<SmtpHost>("\"relay.example.com\"").unwrap(),
            host
        );
        assert!(serde_json::from_str::<SmtpHost>("\"\"").is_err());
    }
}

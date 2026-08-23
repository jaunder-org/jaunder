use std::fmt;
use std::str::FromStr;

use email_address::EmailAddress;
use thiserror::Error;

use crate::display_name::{DisplayName, InvalidDisplayName};
use crate::email::Email;

/// An email address with an optional display name — an RFC 5322 mailbox, e.g.
/// `Jaunder <noreply@localhost>` or a bare `noreply@localhost`.
///
/// Composes the two existing domain newtypes: the normalized address
/// ([`Email`]) and the bounded human label ([`DisplayName`]). Each half is
/// validated by the newtype that owns its rule, so a `Mailbox` is invalid-state-free
/// in both dimensions. Constructed via [`FromStr`] (the single validating chokepoint)
/// or [`Mailbox::new`] from already-valid parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    display_name: Option<DisplayName>,
    address: Email,
}

/// Error returned when a string cannot be parsed as a [`Mailbox`].
#[derive(Debug, Error)]
pub enum InvalidMailbox {
    /// The address half is not a valid email address (carries the parser's reason).
    #[error("invalid mailbox address: {0}")]
    Address(String),
    /// The display-name half is not a valid [`DisplayName`].
    #[error("invalid mailbox display name: {0}")]
    DisplayName(#[from] InvalidDisplayName),
}

impl Mailbox {
    /// Build a `Mailbox` from an already-valid address and optional display name.
    #[must_use]
    pub fn new(address: Email, display_name: Option<DisplayName>) -> Self {
        Self {
            display_name,
            address,
        }
    }

    /// The address half.
    #[must_use]
    pub fn address(&self) -> &Email {
        &self.address
    }

    /// The display name, if any.
    #[must_use]
    pub fn display_name(&self) -> Option<&DisplayName> {
        self.display_name.as_ref()
    }
}

impl FromStr for Mailbox {
    type Err = InvalidMailbox;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse the full RFC 5322 `[display-name] angle-addr` grammar with the
        // `email_address` parser — it correctly handles a quoted local-part that may
        // itself contain `<`/`>`, which a naive angle-bracket string split cannot —
        // then delegate each half to the newtype that owns its rule.
        let parsed: EmailAddress = s
            .parse()
            .map_err(|e: email_address::Error| InvalidMailbox::Address(e.to_string()))?;
        let display = parsed.display_part();
        let display_name = decode_display_name(display)?;
        // `parsed.email()` is the bare addr-spec `email_address` just validated; the
        // `Email` re-parse only normalizes its domain and cannot fail here.
        let Ok(address) = Email::from_str(&parsed.email()) else {
            unreachable!("an email_address-validated addr-spec must re-parse as Email")
        };
        Ok(Self {
            display_name,
            address,
        })
    }
}

fn decode_display_name(display: &str) -> Result<Option<DisplayName>, InvalidMailbox> {
    let trimmed = display.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let decoded = if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        let mut decoded = String::with_capacity(trimmed.len() - 2);
        let mut chars = trimmed[1..trimmed.len() - 1].chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    decoded.push(escaped);
                }
            } else {
                decoded.push(ch);
            }
        }
        decoded
    } else {
        trimmed.to_owned()
    };

    Ok(Some(decoded.parse::<DisplayName>()?))
}

fn needs_quoted_display_name(name: &str) -> bool {
    name.chars()
        .any(|ch| !matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '/' | '=' | '?' | '^' | '_' | '`' | '{' | '|' | '}' | '~' | ' ' | '\t'))
}

fn write_quoted_display_name(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    f.write_str("\"")?;
    for ch in name.chars() {
        if ch == '"' || ch == '\\' {
            f.write_str("\\")?;
        }
        write!(f, "{ch}")?;
    }
    f.write_str("\"")
}

impl fmt::Display for Mailbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.display_name {
            Some(name) if needs_quoted_display_name(name) => {
                write_quoted_display_name(f, name)?;
                write!(f, " <{}>", self.address)
            }
            Some(name) => write!(f, "{name} <{}>", self.address),
            None => write!(f, "{}", self.address),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_parses_named_form() {
        let m: Mailbox = "Jaunder <noreply@localhost>".parse().unwrap();
        assert_eq!(*m.display_name().unwrap(), "Jaunder");
        assert_eq!(m.address(), &"noreply@localhost".parse::<Email>().unwrap());
    }

    #[test]
    fn mailbox_parses_bare_form() {
        let m: Mailbox = "noreply@localhost".parse().unwrap();
        assert!(m.display_name().is_none());
        assert_eq!(m.address(), &"noreply@localhost".parse::<Email>().unwrap());
    }

    #[test]
    fn mailbox_normalizes_address_and_preserves_name() {
        let m: Mailbox = "Foo <A.B@EXAMPLE.COM>".parse().unwrap();
        // `Email` lowercases the domain; the local-part and the display name are preserved.
        assert_eq!(*m.address(), "A.B@example.com");
        assert_eq!(*m.display_name().unwrap(), "Foo");
    }

    #[test]
    fn mailbox_rejects_malformed_address() {
        assert!(matches!(
            "Jaunder <not-an-email>".parse::<Mailbox>(),
            Err(InvalidMailbox::Address(_))
        ));
        assert!("bare-not-an-email".parse::<Mailbox>().is_err());
    }

    #[test]
    fn mailbox_decodes_quoted_display_name() {
        let m: Mailbox = r#""Acme, Inc" <noreply@example.com>"#.parse().unwrap();
        assert_eq!(*m.display_name().unwrap(), "Acme, Inc");
        assert_eq!(m.to_string(), r#""Acme, Inc" <noreply@example.com>"#);
    }

    #[test]
    fn mailbox_decodes_quoted_display_name_escapes() {
        let m: Mailbox = r#""The \"Boss\" \\ Team" <noreply@example.com>"#.parse().unwrap();
        assert_eq!(*m.display_name().unwrap(), r#"The "Boss" \ Team"#);
        assert_eq!(
            m.to_string(),
            r#""The \"Boss\" \\ Team" <noreply@example.com>"#
        );
        assert_eq!(m.to_string().parse::<Mailbox>().unwrap(), m);
    }

    #[test]
    fn mailbox_display_quotes_names_that_are_not_rfc_phrase_atoms() {
        for name in ["Acme, Inc", "Support: Jaunder", "Jaunder (Team)"] {
            let address: Email = "noreply@example.com".parse().unwrap();
            let m = Mailbox::new(address, Some(name.parse().unwrap()));

            assert_eq!(m.to_string(), format!(r#""{name}" <noreply@example.com>"#));
            assert_eq!(m.to_string().parse::<Mailbox>().unwrap(), m);
        }
    }

    #[test]
    fn mailbox_quotes_unquoted_display_names_that_need_it() {
        let m: Mailbox = "Acme, Inc <noreply@example.com>".parse().unwrap();
        assert_eq!(*m.display_name().unwrap(), "Acme, Inc");
        assert_eq!(m.to_string(), r#""Acme, Inc" <noreply@example.com>"#);
        assert_eq!(m.to_string().parse::<Mailbox>().unwrap(), m);
    }

    #[test]
    fn mailbox_rejects_over_long_display_name() {
        use crate::display_name::MAX_DISPLAY_NAME_CHARS;
        let long = "a".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        let input = format!("{long} <a@b.com>");
        assert!(matches!(
            input.parse::<Mailbox>(),
            Err(InvalidMailbox::DisplayName(_))
        ));
    }

    #[test]
    fn mailbox_whitespace_only_name_is_unnamed_not_an_error() {
        let m: Mailbox = "   <a@b.com>".parse().unwrap();
        assert!(m.display_name().is_none());
        assert_eq!(*m.address(), "a@b.com");
    }

    #[test]
    fn mailbox_display_round_trips_both_forms() {
        let named: Mailbox = "Jaunder <noreply@localhost>".parse().unwrap();
        assert_eq!(named.to_string(), "Jaunder <noreply@localhost>");
        assert_eq!(named.to_string().parse::<Mailbox>().unwrap(), named);

        let bare: Mailbox = "noreply@localhost".parse().unwrap();
        assert_eq!(bare.to_string(), "noreply@localhost");
        assert_eq!(bare.to_string().parse::<Mailbox>().unwrap(), bare);
    }

    #[test]
    fn mailbox_round_trips_quoted_local_part_with_angle_bracket() {
        // A quoted local-part may legally contain `<`; the RFC parser handles it and
        // `Display` must be a faithful inverse — a naive `<`/`>` string split would
        // mis-parse the inner bracket.
        let m: Mailbox = "Foo <\"a<b\"@x.com>".parse().unwrap();
        assert_eq!(*m.display_name().unwrap(), "Foo");
        assert_eq!(*m.address(), "\"a<b\"@x.com");
        assert_eq!(m.to_string().parse::<Mailbox>().unwrap(), m);
    }

    #[test]
    fn mailbox_new_and_accessors_round_trip() {
        let address: Email = "a@b.com".parse().unwrap();
        let name: DisplayName = "Ada".parse().unwrap();
        let m = Mailbox::new(address.clone(), Some(name.clone()));
        assert_eq!(m.address(), &address);
        assert_eq!(m.display_name(), Some(&name));

        let unnamed = Mailbox::new(address.clone(), None);
        assert!(unnamed.display_name().is_none());
    }
}

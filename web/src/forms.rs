//! Client-side domain-value form validation (#414): validate a field by parsing input
//! into a domain newtype — the same `FromStr` the typed `#[server]`-arg `Deserialize`
//! routes through — and surface the newtype's own message inline. See ADR (draft):
//! `docs/adr/drafts/client-side-domain-validation.md`.

use std::fmt::Display;
use std::str::FromStr;

/// `None` when `input` parses into the domain newtype `T`; otherwise the newtype's own
/// validation message (its `FromStr::Err` `Display`). The single client/server validation
/// source — re-implementing a newtype's rule in the client is prohibited (#416).
#[must_use]
pub fn field_error<T>(input: &str) -> Option<String>
where
    T: FromStr,
    T::Err: Display,
{
    input.parse::<T>().err().map(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    // `common` has no top-level re-exports — qualify by module.
    use common::password::Password;
    use common::slug::Slug;
    use common::tag::Tag;
    use common::username::Username;

    #[test]
    fn valid_input_is_none() {
        assert_eq!(field_error::<Username>("alice"), None);
        assert_eq!(field_error::<Tag>("rust"), None);
        assert_eq!(field_error::<Slug>("hello"), None);
        assert_eq!(field_error::<Password>("hunter2!"), None); // >= 8 chars
    }

    #[test]
    fn invalid_input_is_the_newtypes_own_message() {
        // The message is exactly the newtype's `FromStr::Err` `Display` — one source of truth.
        let expected = "username must be non-empty and match [a-z0-9_-]+";
        assert_eq!(field_error::<Username>("a b").as_deref(), Some(expected));
        assert_eq!(field_error::<Username>("").as_deref(), Some(expected));
        assert!(field_error::<Password>("short").is_some()); // < 8 chars
        assert!(field_error::<Tag>("Bad Tag").is_some());
    }
}

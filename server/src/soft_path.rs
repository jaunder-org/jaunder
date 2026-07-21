//! `SoftPath<T>` — a route-segment extractor that soft-parses a segment into `T` without
//! axum's pre-handler 400 (#504). A parse miss reaches the handler as `None`, which the
//! handler renders as the SPA shell / a soft 404 — the deliberate projector-vs-atompub
//! boundary split (ADR-0063 §4). A strict typed extractor (`Path<Username>`, used by the
//! atompub API handlers) would instead reject a malformed segment with a 400 before the
//! handler runs; these public/projector routes want the soft fall-through.

use std::str::FromStr;

use serde::{Deserialize, Deserializer};

/// A path segment soft-parsed into `T`: `Some(t)` on success, `None` on a parse miss.
///
/// Being `Deserialize`-based (via `String::deserialize`), it composes anywhere `Path<String>`
/// does — a bare segment, a tuple element (including mixed with `i32`/`u32` date segments), or
/// a named-struct field — and its deserialization **never errors**, so a malformed segment is
/// `None`, not a pre-handler 400.
///
/// Accessors follow the ADR-0063 domain-newtype convention: `value()` borrows, and the owned
/// unwrap is `From<Self> for <inner>` (here the inner is `Option<T>`), invoked as `.into()`.
pub struct SoftPath<T>(Option<T>);

impl<T: FromStr> SoftPath<T> {
    /// Soft-parse `s`: `Some` on success, `None` on a parse miss. The one soft-parse
    /// chokepoint — [`Deserialize`] and test fixtures both route through it.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        SoftPath(s.parse::<T>().ok())
    }
}

impl<T> SoftPath<T> {
    /// The parsed value by reference, or `None` on a miss.
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

/// Owned unwrap to the inner `Option<T>` — the ADR-0063 `From<Self> for <inner>` convention.
/// Invoked as `soft.into()` (e.g. `let Some(x) = soft.into() else { … }`).
impl<T> From<SoftPath<T>> for Option<T> {
    fn from(soft: SoftPath<T>) -> Self {
        soft.0
    }
}

impl<'de, T: FromStr> Deserialize<'de> for SoftPath<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize as a plain `String` (axum's path deserializer drives this exactly like
        // `Path<String>`), then soft-parse — a miss is `None`, NOT an error, so no 400.
        let s = String::deserialize(deserializer)?;
        Ok(SoftPath::parse(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::username::Username;

    #[test]
    fn parse_valid_is_some_and_unwraps() {
        let sp = SoftPath::<Username>::parse("alice");
        assert_eq!(sp.value().map(AsRef::as_ref), Some("alice"));
        let owned: Option<Username> = sp.into();
        assert_eq!(owned.as_deref(), Some("alice"));
    }

    #[test]
    fn parse_invalid_is_none_not_error() {
        let sp = SoftPath::<Username>::parse("not a valid name!!");
        assert!(sp.value().is_none());
        let owned: Option<Username> = sp.into();
        assert!(owned.is_none());
    }

    #[test]
    fn deserialize_never_errors_valid_or_invalid() {
        // The load-bearing property: deserialize SUCCEEDS for both, storing Some/None —
        // never an error (which, on the path, would become a pre-handler 400).
        let ok: SoftPath<Username> = serde_json::from_str("\"alice\"").unwrap();
        assert!(ok.value().is_some());
        let miss: SoftPath<Username> = serde_json::from_str("\"bad name!\"").unwrap();
        assert!(miss.value().is_none());
    }
}

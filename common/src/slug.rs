use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Maximum slug length, in Unicode scalar values counted after NFC
/// normalization. Bounds the percent-encoded URL and the stored value (CJK
/// inflates ~3 bytes/char in UTF-8, more percent-encoded).
pub const MAX_SLUG_CHARS: usize = 80;

/// A validated post slug: NFC-normalized, Unicode-lowercased, made of Unicode
/// letters/digits (`char::is_alphanumeric`) and `-`, at most [`MAX_SLUG_CHARS`].
///
/// Constructed via [`FromStr`], the single chokepoint both slug *generation* and
/// inbound *URL resolution* funnel through; it normalizes so the stored form and
/// an inbound lookup compare byte-for-byte regardless of the request's case or
/// normal form. The `try_from`/`into` serde bridge routes (de)serialization
/// through that same validation, so a `Slug` serializes as a plain string and
/// rejects invalid input on the wire — safe as a (de)serialized DTO field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Slug(String);

/// Error returned when a string cannot be parsed as a [`Slug`].
#[derive(Debug, Error)]
#[error(
    "slug must be non-empty, at most {MAX_SLUG_CHARS} characters, and contain only Unicode letters/digits and '-'"
)]
pub struct InvalidSlug;

impl FromStr for Slug {
    type Err = InvalidSlug;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize so stored slugs and inbound-URL lookups compare consistently
        // regardless of case or Unicode normal form: lowercase (full-Unicode),
        // then NFC-compose. Idempotent on an already-stored slug, so the read-path
        // re-parse (storage::helpers) and inbound lookups agree on bytes.
        let normalized: String = s.to_lowercase().nfc().collect();
        let mut chars = normalized.chars();
        // First character must be a Unicode letter or digit (no leading '-').
        let first = chars.next().ok_or(InvalidSlug)?;
        if !first.is_alphanumeric() {
            return Err(InvalidSlug);
        }
        // Remaining characters: Unicode letters/digits or hyphen.
        if !chars.all(|c| c.is_alphanumeric() || c == '-') {
            return Err(InvalidSlug);
        }
        if normalized.chars().count() > MAX_SLUG_CHARS {
            return Err(InvalidSlug);
        }
        Ok(Slug(normalized))
    }
}

impl TryFrom<String> for Slug {
    type Error = InvalidSlug;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Slug> for String {
    fn from(value: Slug) -> Self {
        value.0
    }
}

impl Slug {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Converts a title to a slug: NFC-normalized, Unicode-lowercased, keeping only
/// `char::is_alphanumeric()` characters and collapsing other runs into single
/// hyphens, truncated to [`MAX_SLUG_CHARS`].
///
/// Never fails: when nothing usable remains (emoji/symbol-only, untitled) it
/// returns the bare fallback `"post"`, and the caller's per-author-per-day
/// collision retry disambiguates. The result is already normalized, so feeding
/// it back through [`Slug::from_str`] is idempotent.
#[must_use]
pub fn slugify_title(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_dash = false;

    for ch in title.to_lowercase().nfc() {
        if ch.is_alphanumeric() {
            slug.push(ch);
            previous_was_dash = false;
        } else if !slug.is_empty() && !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    // Truncate to the cap, then trim a trailing '-' (present originally or exposed
    // by truncation). `trim_end_matches` avoids an explicit pop loop.
    let capped: String = slug
        .trim_end_matches('-')
        .chars()
        .take(MAX_SLUG_CHARS)
        .collect();
    let capped = capped.trim_end_matches('-');

    if capped.is_empty() {
        "post".to_owned()
    } else {
        capped.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_accepts_ascii_and_unicode_lowercasing() {
        assert_eq!(
            "hello-world".parse::<Slug>().unwrap().as_str(),
            "hello-world"
        );
        assert_eq!(
            "my-post-2024".parse::<Slug>().unwrap().as_str(),
            "my-post-2024"
        );
        // Uppercase is now accepted (lowercased), not rejected.
        assert_eq!("Héllo".parse::<Slug>().unwrap().as_str(), "héllo");
        assert_eq!("日本語".parse::<Slug>().unwrap().as_str(), "日本語");
        assert_eq!("Москва".parse::<Slug>().unwrap().as_str(), "москва");
        assert_eq!("café".parse::<Slug>().unwrap().as_str(), "café");
    }

    #[test]
    fn slug_normalizes_nfd_input_to_nfc() {
        // "cafe" + combining acute (NFD) normalizes to NFC "café" so an
        // NFD-encoded inbound request matches an NFC-stored slug.
        let nfd = "cafe\u{0301}";
        assert_eq!(nfd.parse::<Slug>().unwrap().as_str(), "café");
        assert_eq!(
            nfd.parse::<Slug>().unwrap(),
            "café".parse::<Slug>().unwrap()
        );
    }

    #[test]
    fn slug_rejects_invalid_values() {
        assert!("".parse::<Slug>().is_err()); // empty
        assert!("-hello".parse::<Slug>().is_err()); // leading hyphen
        assert!("hello world".parse::<Slug>().is_err()); // space
        assert!("hello_world".parse::<Slug>().is_err()); // underscore
        assert!("hello@world".parse::<Slug>().is_err()); // symbol
        assert!("🚀".parse::<Slug>().is_err()); // emoji is a Unicode Symbol, not alnum
    }

    #[test]
    fn slug_enforces_length_cap() {
        let max: String = "a".repeat(MAX_SLUG_CHARS);
        assert!(max.parse::<Slug>().is_ok());
        let over: String = "a".repeat(MAX_SLUG_CHARS + 1);
        assert!(over.parse::<Slug>().is_err());
    }

    #[test]
    fn slug_display_returns_inner_string() {
        let s: Slug = "my-post".parse().unwrap();
        assert_eq!(s.to_string(), "my-post");
        assert_eq!(s.as_str(), "my-post");
    }

    #[test]
    fn slug_serde_serializes_as_plain_string_and_validates_on_deserialize() {
        let s: Slug = "my-post".parse().unwrap();
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"my-post\"");
        assert_eq!(
            serde_json::from_str::<Slug>("\"my-post\"").unwrap(),
            "my-post".parse::<Slug>().unwrap()
        );
        // Invalid input is rejected at deserialize time.
        assert!(serde_json::from_str::<Slug>("\"Bad Slug\"").is_err());
    }

    #[test]
    fn slugify_title_preserves_unicode_lowercased() {
        assert_eq!(slugify_title("Café"), "café");
        assert_eq!(slugify_title("日本語"), "日本語");
        assert_eq!(slugify_title("Москва"), "москва");
        assert_eq!(
            slugify_title("Hello, World from Rust"),
            "hello-world-from-rust"
        );
        assert_eq!(slugify_title("  ---Héllo!!!  "), "héllo");
        assert_eq!(slugify_title("Rust"), "rust");
    }

    #[test]
    fn slugify_title_falls_back_to_post_when_no_letters() {
        assert_eq!(slugify_title("!!!"), "post");
        assert_eq!(slugify_title("—"), "post");
        assert_eq!(slugify_title("🚀🎉"), "post");
        assert_eq!(slugify_title("   "), "post");
    }

    #[test]
    fn slugify_title_truncates_to_cap_on_char_boundary() {
        let long = "あ".repeat(200);
        let s = slugify_title(&long);
        assert_eq!(s.chars().count(), MAX_SLUG_CHARS);
        assert!(s.parse::<Slug>().is_ok());

        // Truncation that lands on a '-' separator trims the trailing dash so the
        // result never ends with one.
        let with_sep = format!("{} b", "a".repeat(MAX_SLUG_CHARS - 1));
        let s2 = slugify_title(&with_sep);
        assert_eq!(s2, "a".repeat(MAX_SLUG_CHARS - 1));
        assert!(!s2.ends_with('-'));
    }
}

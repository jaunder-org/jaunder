use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// Maximum slug length, in Unicode scalar values counted after NFC
/// normalization. Bounds the percent-encoded URL and the stored value (CJK
/// inflates ~3 bytes/char in UTF-8, more percent-encoded).
pub const MAX_SLUG_CHARS: usize = 80;

/// A validated post slug: NFC-normalized, Unicode-lowercased, made of grapheme
/// clusters whose base is a Unicode letter/digit (carrying any attached combining
/// marks) and `-`, at most [`MAX_SLUG_CHARS`] scalars.
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
    "slug must be non-empty, at most {MAX_SLUG_CHARS} characters, and contain only Unicode letters/digits (with their combining marks) and '-'"
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
        if normalized.chars().count() > MAX_SLUG_CHARS {
            return Err(InvalidSlug);
        }
        let mut graphemes = normalized.graphemes(true);
        // First grapheme must be a letter/digit-based cluster (no leading '-' or
        // combining mark).
        let first = graphemes.next().ok_or(InvalidSlug)?;
        if !base_is_alphanumeric(first) {
            return Err(InvalidSlug);
        }
        // Remaining graphemes: a hyphen, or a letter/digit-based cluster (its
        // attached combining marks come with it).
        if !graphemes.all(|g| g == "-" || base_is_alphanumeric(g)) {
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

/// A grapheme cluster is kept in a slug iff its base scalar is a Unicode letter
/// or digit (`char::is_alphanumeric`). Attached combining marks — vowel signs,
/// viramas, harakat, nuktas — ride along with the base; a standalone mark, a
/// symbol, or an emoji has a non-alphanumeric base and is dropped. This one rule
/// is shared by generation and the `from_str` chokepoint so they always agree.
fn base_is_alphanumeric(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(char::is_alphanumeric)
}

/// Converts a title to a slug: NFC-normalized, Unicode-lowercased, keeping
/// grapheme clusters whose base is a letter/digit (with their attached combining
/// marks) and collapsing other runs into single hyphens, truncated on a cluster
/// boundary to [`MAX_SLUG_CHARS`] scalars.
///
/// Never fails: when nothing usable remains (emoji/symbol-only, untitled) it
/// returns the bare fallback `"post"`, and the caller's per-author-per-day
/// collision retry disambiguates. The result is already normalized, so feeding
/// it back through [`Slug::from_str`] is idempotent.
#[must_use]
pub fn slugify_title(title: &str) -> String {
    let normalized: String = title.to_lowercase().nfc().collect();

    let mut slug = String::new();
    let mut previous_was_dash = false;
    for g in normalized.graphemes(true) {
        if base_is_alphanumeric(g) {
            slug.push_str(g);
            previous_was_dash = false;
        } else if !slug.is_empty() && !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    // Trim a trailing '-', then truncate on a grapheme boundary: a cluster (base
    // + its marks) is never split, and the result stays within the
    // MAX_SLUG_CHARS scalar budget that `from_str` enforces.
    let trimmed = slug.trim_end_matches('-');
    let mut capped = String::new();
    let mut count = 0usize;
    for g in trimmed.graphemes(true) {
        let glen = g.chars().count();
        if count + glen > MAX_SLUG_CHARS {
            break;
        }
        capped.push_str(g);
        count += glen;
    }
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
    fn slug_preserves_indic_conjunct_marks() {
        // Virama/pulli (Mn, not Alphabetic) were dropped, breaking conjuncts.
        for word in ["नमस्ते", "हिन्दी", "தமிழ்"] {
            let nfc: String = word.to_lowercase().nfc().collect();
            assert_eq!(
                slugify_title(word),
                nfc,
                "slugify dropped a mark in {word:?}"
            );
            // Chokepoint agreement: the generated slug re-parses to itself.
            assert_eq!(slugify_title(word).parse::<Slug>().unwrap().as_str(), nfc);
        }
    }

    #[test]
    fn slug_keeps_alphabetic_marks_regression() {
        // Arabic harakat already survived (Other_Alphabetic); keep it so.
        let arabic = "مَرْحَبًا";
        let nfc: String = arabic.to_lowercase().nfc().collect();
        assert_eq!(slugify_title(arabic), nfc);
        assert_eq!(arabic.parse::<Slug>().unwrap().as_str(), nfc);
    }

    #[test]
    fn slug_drops_standalone_mark_and_rejects_leading_mark() {
        // A lone virama (no base) is degenerate: generation lands on the fallback...
        assert_eq!(slugify_title("\u{094D}"), "post");
        // ...and from_str rejects a slug that starts with a combining mark...
        assert!("\u{094D}a".parse::<Slug>().is_err());
        // ...while a mark attached to a base is accepted.
        assert!("क\u{093E}".parse::<Slug>().is_ok());
    }

    #[test]
    fn slugify_truncates_on_grapheme_boundary_within_cap() {
        // 2-scalar clusters (consonant + vowel sign) well over the cap.
        let title = "क\u{093E}".repeat(MAX_SLUG_CHARS); // 2*MAX scalars
        let slug = slugify_title(&title);
        assert!(slug.chars().count() <= MAX_SLUG_CHARS); // cap honored
        assert_eq!(slug.chars().count() % 2, 0); // never split a 2-scalar cluster
        assert!(slug.parse::<Slug>().is_ok()); // still valid
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

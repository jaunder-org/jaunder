use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// Maximum post-summary length, in Unicode scalar values.
pub const MAX_POST_SUMMARY_CHARS: usize = 500;

/// A validated post summary/excerpt: outer whitespace trimmed, non-empty, at most
/// [`MAX_POST_SUMMARY_CHARS`] scalars; inner whitespace/newlines and case preserved (a
/// summary is free-form prose, not a normalized identifier).
///
/// The **public** construction doors — [`FromStr`] and the serde/sqlx bridges the
/// `StrNewtype` derive emits — enforce the full invariant (non-empty **and** ≤ cap), so
/// interior code works only with already-valid summaries and an invalid string is rejected
/// at the boundary and on the wire. [`PostSummary::truncated`] is an *internal trusted*
/// door that guarantees only the length half (see its docs). Absence of a summary is
/// modeled by `Option<PostSummary>` at the boundary, so `FromStr` rejecting the empty
/// string means an empty wire value is rejected and clearing goes through omission
/// (`None`). No `Hash` — a `PostSummary` is never a map/set key.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PostSummary(String);

/// Error returned when a string cannot be parsed as a [`PostSummary`].
#[derive(Debug, Error)]
#[error("post summary must be non-empty and at most {MAX_POST_SUMMARY_CHARS} characters")]
pub struct InvalidPostSummary;

impl FromStr for PostSummary {
    type Err = InvalidPostSummary;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_POST_SUMMARY_CHARS {
            return Err(InvalidPostSummary);
        }
        Ok(PostSummary(trimmed.to_owned()))
    }
}

impl PostSummary {
    /// Build a `PostSummary` from an internally-derived, **non-empty** label (a post's
    /// first body line, title, or slug), truncating to [`MAX_POST_SUMMARY_CHARS`] scalars.
    ///
    /// Infallible length-validated **trusted** door — the string analog of
    /// `NumNewtype::clamped` and the `RenderedHtml::from_trusted` model. It guarantees the
    /// length cap but **not** non-emptiness (that half of the invariant is enforced only
    /// by [`FromStr`]/serde); callers must pass non-empty input, which the `debug_assert!`
    /// pins in test/debug builds. The only callers are the two label producers —
    /// `storage::PostRecord::fallback_summary_label` and
    /// `common::render::derive_post_metadata` — each of which falls back through a
    /// non-empty body line, title, then slug.
    ///
    /// The cut today is a raw scalar-count boundary (`chars().take(MAX)`), which can slice
    /// mid-word; #564 tracks making it word-/sentence-aware (the cap stays the ceiling).
    #[must_use]
    pub fn truncated(s: &str) -> Self {
        let trimmed = s.trim();
        debug_assert!(
            !trimmed.is_empty(),
            "PostSummary::truncated requires non-empty input"
        );
        PostSummary(trimmed.chars().take(MAX_POST_SUMMARY_CHARS).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_trims_preserving_inner_and_case() {
        assert_eq!(
            "  Hello  World  ".parse::<PostSummary>().unwrap(),
            "Hello  World"
        );
        // Inner newlines are preserved (summaries are multi-line free-form prose).
        assert_eq!(
            "line1\nline2".parse::<PostSummary>().unwrap(),
            "line1\nline2"
        );
        // Unicode is preserved as-is (no normalization).
        assert_eq!("Пример".parse::<PostSummary>().unwrap(), "Пример");
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert!("".parse::<PostSummary>().is_err());
        assert!("   \t\n".parse::<PostSummary>().is_err());
    }

    #[test]
    fn enforces_length_cap_on_scalars_post_trim() {
        let max: String = "a".repeat(MAX_POST_SUMMARY_CHARS);
        assert!(max.parse::<PostSummary>().is_ok());
        let over: String = "a".repeat(MAX_POST_SUMMARY_CHARS + 1);
        assert!(over.parse::<PostSummary>().is_err());
        // The cap counts scalars post-trim, so surrounding whitespace does not push an
        // otherwise-valid summary over the limit.
        let padded = format!("  {}  ", "a".repeat(MAX_POST_SUMMARY_CHARS));
        assert!(padded.parse::<PostSummary>().is_ok());
    }

    #[test]
    fn serde_serializes_plain_string_and_validates_on_deserialize() {
        let s: PostSummary = "Blurb".parse().unwrap();
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"Blurb\"");
        assert_eq!(serde_json::from_str::<PostSummary>("\"Blurb\"").unwrap(), s);
        // Invalid input is rejected at deserialize time.
        assert!(serde_json::from_str::<PostSummary>("\"\"").is_err());
        let over = format!("\"{}\"", "a".repeat(MAX_POST_SUMMARY_CHARS + 1));
        assert!(serde_json::from_str::<PostSummary>(&over).is_err());
    }

    #[test]
    fn truncated_trims_and_caps_at_char_boundary() {
        // Under cap: unchanged (but trimmed).
        assert_eq!(PostSummary::truncated("  hi  "), "hi");
        // Over cap: truncated to exactly MAX scalars, no panic on multibyte input.
        let over: String = "é".repeat(MAX_POST_SUMMARY_CHARS + 50);
        let t = PostSummary::truncated(&over);
        assert_eq!(t.chars().count(), MAX_POST_SUMMARY_CHARS);
    }

    #[test]
    #[should_panic(expected = "non-empty")]
    fn truncated_debug_asserts_non_empty() {
        // Documents the caller-trusted precondition (fires in test/debug builds).
        let _ = PostSummary::truncated("   ");
    }
}

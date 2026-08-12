use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

use crate::post_body::PostBody;
use crate::post_title::PostTitle;
use crate::slug::Slug;

/// Maximum post-summary length, in Unicode scalar values.
pub const MAX_POST_SUMMARY_CHARS: usize = 500;

/// A validated post summary/excerpt: outer whitespace trimmed, non-empty, at most
/// [`MAX_POST_SUMMARY_CHARS`] scalars; inner whitespace/newlines and case preserved (a
/// summary is free-form prose, not a normalized identifier).
///
/// The **public** construction doors — [`FromStr`] and the serde/sqlx bridges the
/// `StrNewtype` derive emits — enforce the full invariant (non-empty **and** ≤ cap), so
/// interior code works only with already-valid summaries and an invalid string is rejected
/// at the boundary and on the wire. [`PostSummary::truncated`] is an *internal* door that
/// enforces only the length half, because its input — a [`SummarySeed`] — already carries
/// non-blankness as a type (#830). Absence of a summary is
/// modeled by `Option<PostSummary>` at the boundary, so `FromStr` rejecting the empty
/// string means an empty wire value is rejected and clearing goes through omission
/// (`None`). No `Hash` — nothing hashes a `PostSummary`; ordering is emitted by the
/// trailer (ADR-0063 §2).
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

/// How much of a body line may seed a summary, in Unicode scalar values.
pub const MAX_BODY_LINE_SEED_CHARS: usize = 100;

/// A summary label already known to be non-blank.
///
/// Its constructors are **infallible because each source proves the invariant**: a
/// [`Slug`] and a [`PostTitle`] are non-blank by construction, and a body line is
/// selected only when non-blank. That is what lets [`PostSummary::truncated`] be a
/// plain length-capping door instead of a trusted one carrying a `debug_assert` the
/// caller had to remember to satisfy (#830).
///
/// Only the length half of the summary invariant is left for `truncated` to enforce,
/// and it coerces rather than rejects — appropriate for an internally derived value
/// (ADR-0063 §2's lossy door), unlike a *submitted* summary, which goes through the
/// validating [`FromStr`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummarySeed(String);

impl SummarySeed {
    /// A slug is non-blank by construction — `Slug::from_str` rejects the empty string.
    #[must_use]
    pub fn from_slug(slug: &Slug) -> Self {
        Self(slug.to_string())
    }

    /// A [`PostTitle`] is non-blank by construction (#830).
    #[must_use]
    pub fn from_title(title: &PostTitle) -> Self {
        Self(title.to_string())
    }

    /// The first non-blank line of `body`, trimmed and capped at
    /// [`MAX_BODY_LINE_SEED_CHARS`].
    ///
    /// **Total.** [`PostBody`]'s `FromStr` rejects a body whose every line is blank
    /// (#811), so such a line always exists — this constructor cannot decline, and
    /// the caller needs no impossible-case arm.
    ///
    /// The search is written to *be* total rather than to search and then prove the
    /// search succeeded: `trim_start` crosses newlines, so it skips the leading blank
    /// lines wholesale and lands on the first non-whitespace character, which is by
    /// definition inside the line we want. `split_once` then ends that line, and its
    /// `None` arm is not a failure — it means the line runs to the end of the body.
    #[must_use]
    pub fn first_body_line(body: &PostBody) -> Self {
        let rest = body.trim_start();
        let line = rest.split_once('\n').map_or(rest, |(first, _)| first);
        Self(
            line.trim_end()
                .chars()
                .take(MAX_BODY_LINE_SEED_CHARS)
                .collect(),
        )
    }
}

impl PostSummary {
    /// Length-cap an already-non-blank [`SummarySeed`] into a `PostSummary`.
    ///
    /// Infallible, and it needs no emptiness check: the seed carries that half of the
    /// invariant as a type. The seed is also already trimmed by its source, so this
    /// door only caps. The one caller is the label producer
    /// `storage::PostRecord::fallback_summary_label`.
    ///
    /// The cut today is a raw scalar-count boundary (`chars().take(MAX)`), which can slice
    /// mid-word; #564 tracks making it word-/sentence-aware (the cap stays the ceiling).
    #[must_use]
    pub fn truncated(seed: &SummarySeed) -> Self {
        PostSummary(seed.0.chars().take(MAX_POST_SUMMARY_CHARS).collect())
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
    fn truncated_caps_at_char_boundary_from_a_title_seed() {
        // A title is the only unbounded seed — a slug caps at 80 and a body line at
        // MAX_BODY_LINE_SEED_CHARS — so it is what keeps the 500-scalar cap reachable.
        let under: PostTitle = "hi".parse().unwrap();
        assert_eq!(
            PostSummary::truncated(&SummarySeed::from_title(&under)),
            "hi"
        );

        let over: PostTitle = "é".repeat(MAX_POST_SUMMARY_CHARS + 50).parse().unwrap();
        let capped = PostSummary::truncated(&SummarySeed::from_title(&over));
        assert_eq!(capped.chars().count(), MAX_POST_SUMMARY_CHARS);
    }

    #[test]
    fn first_body_line_finds_the_first_non_blank_line_and_caps_it() {
        // Leading blank lines, then a line padded both sides: the seed is the trimmed
        // line, and the blank lines are skipped rather than declined.
        let body = crate::test_support::parse_post_body("\n\n   \n  hello  \nsecond\n");
        let seed = SummarySeed::first_body_line(&body);
        assert_eq!(PostSummary::truncated(&seed), "hello");

        // The body-line seed carries its own, tighter cap.
        let long = crate::test_support::parse_post_body(&"x".repeat(MAX_POST_SUMMARY_CHARS));
        let seed = SummarySeed::first_body_line(&long);
        assert_eq!(
            PostSummary::truncated(&seed).chars().count(),
            MAX_BODY_LINE_SEED_CHARS
        );
    }

    #[test]
    fn first_body_line_takes_a_body_with_no_trailing_newline() {
        // The `split_once` `None` arm: the first line runs to the end of the body.
        let body = crate::test_support::parse_post_body("only line");
        assert_eq!(
            PostSummary::truncated(&SummarySeed::first_body_line(&body)),
            "only line"
        );
    }

    #[test]
    fn first_body_line_strips_a_carriage_return() {
        // CRLF bodies: `split_once('\n')` leaves the `\r`, and `trim_end` takes it.
        let body = crate::test_support::parse_post_body("\r\n  hi  \r\nnext\r\n");
        assert_eq!(
            PostSummary::truncated(&SummarySeed::first_body_line(&body)),
            "hi"
        );
    }

    #[test]
    fn seed_from_slug_is_the_always_available_fallback() {
        let slug: Slug = "my-slug".parse().unwrap();
        assert_eq!(
            PostSummary::truncated(&SummarySeed::from_slug(&slug)),
            "my-slug"
        );
    }
}

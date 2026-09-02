use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

use crate::post_body::PostBody;
use crate::post_title::PostTitle;
use crate::text;

/// Maximum post-summary length, in Unicode scalar values.
pub const MAX_POST_SUMMARY_CHARS: usize = 500;

/// A validated post summary/excerpt: outer whitespace trimmed, non-empty, at most
/// [`MAX_POST_SUMMARY_CHARS`] scalars; inner whitespace/newlines and case preserved (a
/// summary is free-form prose, not a normalized identifier).
///
/// The **public** construction doors — [`FromStr`] and the serde/sqlx bridges the
/// `StrNewtype` derive emits — enforce the full invariant (non-empty **and** ≤ cap), so
/// submitted invalid strings are rejected at the boundary and on the wire. The internal
/// derived doors — [`PostSummary::from_title`] and [`PostSummary::from_body_line`] —
/// coerce already-non-blank sources into valid summaries with boundary-aware caps.
/// Absence of a summary is modeled by `Option<PostSummary>` at the boundary, so
/// `FromStr` rejecting the empty string means an empty wire value is rejected and
/// clearing goes through omission (`None`). No `Hash` — nothing hashes a `PostSummary`;
/// ordering is emitted by the trailer (ADR-0063 §2).
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PostSummary(String);

/// Error returned when a string cannot be parsed as a [`PostSummary`].
#[derive(Debug, Error)]
#[error("post summary must be non-empty and at most {MAX_POST_SUMMARY_CHARS} characters")]
pub struct InvalidPostSummary;

impl FromStr for PostSummary {
    type Err = InvalidPostSummary;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed =
            text::bounded_non_empty(s, MAX_POST_SUMMARY_CHARS).ok_or(InvalidPostSummary)?;
        Ok(PostSummary(trimmed.to_owned()))
    }
}

/// How much of a body line may seed a summary, in Unicode scalar values.
pub const MAX_BODY_LINE_SEED_CHARS: usize = 100;

impl PostSummary {
    /// Derive a summary from a non-blank title, capping at a sentence or word boundary
    /// when possible while keeping the summary invariant infallible.
    #[must_use]
    pub fn from_title(title: &PostTitle) -> Self {
        Self(truncate_at_text_boundary(
            title.as_ref(),
            MAX_POST_SUMMARY_CHARS,
        ))
    }

    /// Derive a summary from the first non-blank body line.
    ///
    /// **Total.** [`PostBody`]'s `FromStr` rejects a body whose every line is blank
    /// (#811), so such a line always exists.
    #[must_use]
    pub fn from_body_line(body: &PostBody) -> Self {
        let rest = body.trim_start();
        let line = rest.split_once('\n').map_or(rest, |(first, _)| first);
        Self(truncate_at_text_boundary(
            line.trim_end(),
            MAX_BODY_LINE_SEED_CHARS,
        ))
    }
}

/// Truncate an already-selected derived-text seed to `max_scalars` Unicode scalar
/// values.
///
/// The cut prefers the last complete sentence (`.`, `!`, or `?`) within the cap,
/// then the last Unicode whitespace boundary within the cap, and only then the hard
/// scalar cap. Boundary cuts trim trailing whitespace and must leave a non-empty
/// prefix; the hard cap walks `char`s, so it never splits a UTF-8 scalar.
pub(crate) fn truncate_at_text_boundary(input: &str, max_scalars: usize) -> String {
    if input.chars().count() <= max_scalars {
        return input.to_owned();
    }

    let hard_cap: String = input.chars().take(max_scalars).collect();
    let sentence = hard_cap
        .char_indices()
        .rev()
        .find_map(|(idx, ch)| matches!(ch, '.' | '!' | '?').then_some(idx + ch.len_utf8()))
        .and_then(|end| non_empty_trimmed_prefix(&hard_cap[..end]));
    if let Some(prefix) = sentence {
        return prefix;
    }

    let word = hard_cap
        .char_indices()
        .rev()
        .find_map(|(idx, ch)| ch.is_whitespace().then_some(idx))
        .and_then(|end| non_empty_trimmed_prefix(&hard_cap[..end]));
    if let Some(prefix) = word {
        return prefix;
    }

    hard_cap
}

fn non_empty_trimmed_prefix(prefix: &str) -> Option<String> {
    let trimmed = prefix.trim_end();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
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
    fn invalid_error_display_is_the_public_message() {
        assert_eq!(
            InvalidPostSummary.to_string(),
            "post summary must be non-empty and at most 500 characters"
        );
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
    fn derived_title_summary_prefers_sentence_boundary() {
        let prefix = "A complete sentence.";
        let filler = format!(" {}", "word".repeat(MAX_POST_SUMMARY_CHARS));
        let title: PostTitle = format!("{prefix}{filler}").parse().unwrap();

        assert_eq!(PostSummary::from_title(&title), prefix);
    }

    #[test]
    fn derived_title_summary_falls_back_to_word_boundary() {
        let title: PostTitle = format!("{} finalword", "word ".repeat(MAX_POST_SUMMARY_CHARS / 5))
            .parse()
            .unwrap();
        let summary = PostSummary::from_title(&title);

        assert!(summary.chars().count() <= MAX_POST_SUMMARY_CHARS);
        assert!(!summary.ends_with("finalword"));
        assert!(!summary.ends_with(' '));
    }

    #[test]
    fn derived_title_summary_hard_caps_one_long_token_without_splitting_utf8() {
        let title: PostTitle = "é".repeat(MAX_POST_SUMMARY_CHARS + 50).parse().unwrap();
        let summary = PostSummary::from_title(&title);

        assert_eq!(summary.chars().count(), MAX_POST_SUMMARY_CHARS);
        assert!(summary.chars().all(|c| c == 'é'));
    }

    #[test]
    fn derived_body_summary_prefers_boundary_within_body_line_cap() {
        let body = crate::test_support::parse_post_body(&format!(
            "{} trailingword\nsecond line",
            "body word ".repeat(MAX_BODY_LINE_SEED_CHARS / 10)
        ));
        let summary = PostSummary::from_body_line(&body);

        assert!(summary.chars().count() <= MAX_BODY_LINE_SEED_CHARS);
        assert!(!summary.ends_with("trailingword"));
        assert!(!summary.ends_with(' '));
    }

    #[test]
    fn submitted_over_cap_summary_still_rejects() {
        let over = "a".repeat(MAX_POST_SUMMARY_CHARS + 1);

        assert!(over.parse::<PostSummary>().is_err());
    }

    #[test]
    fn first_body_line_takes_a_body_with_no_trailing_newline() {
        let body = crate::test_support::parse_post_body("only line");
        assert_eq!(PostSummary::from_body_line(&body), "only line");
    }

    #[test]
    fn first_body_line_strips_a_carriage_return() {
        let body = crate::test_support::parse_post_body("\r\n  hi  \r\nnext\r\n");
        assert_eq!(PostSummary::from_body_line(&body), "hi");
    }
}

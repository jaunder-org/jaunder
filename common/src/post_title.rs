use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A Post's authored title: outer non-line-breaking whitespace trimmed, non-empty,
/// and one logical source line. Case and internal non-line-breaking whitespace
/// are preserved (a title is human prose, not an identifier).
///
/// Constructed via [`FromStr`] — the single validating chokepoint, so a blank title is
/// **unrepresentable** rather than something every call site must remember to filter
/// (#830). An *absent* title is `None`: the field is `Option<PostTitle>` throughout,
/// and blank `AtomPub` input means absent (its mapping uses
/// [`PostTitle::parse_optional`] to distinguish blank from invalid). The rest of the ADR-0063 string-newtype trailer
/// (`Display`, `AsRef`/`Borrow`/`Deref<str>`, owned-`String` conversions,
/// `PartialEq<str>`, ordering, and the validating serde and sqlx bridges) is generated
/// by `#[derive(StrNewtype)]`, so a `PostTitle` serializes as a plain string and
/// rejects blank input on the wire and on decode.
///
/// **No length bound** — unlike [`crate::session_label::SessionLabel`], a title is
/// unbounded prose, and bounding it is a separate derived-summary decision.
/// [`crate::post_summary::PostSummary::from_title`] therefore keeps
/// `MAX_POST_SUMMARY_CHARS` reachable.
///
/// No `Hash` — nothing hashes a `PostTitle`; ordering is emitted by the trailer
/// (ADR-0063 §2), matching `SessionLabel` and `PostSummary`.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PostTitle(String);

/// Error returned when a string cannot be parsed as a [`PostTitle`].
#[derive(Debug, Error)]
#[error("post title must be non-empty and contain no line breaks")]
pub struct InvalidPostTitle;

impl PostTitle {
    /// Parse a title at a boundary where absence and non-line-breaking blank
    /// input both mean no title. Authored line breaks still reject, including
    /// whitespace-only input containing one.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPostTitle`] when the source contains a line break.
    pub fn parse_optional(source: &str) -> Result<Option<Self>, InvalidPostTitle> {
        if source.trim().is_empty() && !source.chars().any(is_line_separator) {
            Ok(None)
        } else {
            source.parse().map(Some)
        }
    }
}

fn is_line_separator(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

impl FromStr for PostTitle {
    type Err = InvalidPostTitle;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Check before trimming: even an edge line break is authored source,
        // not surrounding whitespace we may silently discard.
        if s.chars().any(is_line_separator) {
            return Err(InvalidPostTitle);
        }
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(InvalidPostTitle);
        }
        Ok(PostTitle(trimmed.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_title_trims_outer_whitespace_preserving_inner_and_case() {
        assert_eq!(
            "  Hello  World  ".parse::<PostTitle>().unwrap(),
            "Hello  World"
        );
        // Unicode is preserved as-is (no lowercasing/normalization).
        assert_eq!("Москва".parse::<PostTitle>().unwrap(), "Москва");
        assert_eq!(
            " \tHello\t World\t ".parse::<PostTitle>().unwrap(),
            "Hello\t World"
        );
        assert_eq!(
            "Hello<br>World".parse::<PostTitle>().unwrap(),
            "Hello<br>World"
        );
    }

    #[test]
    fn post_title_rejects_authored_line_separators_at_every_position() {
        for separator in [
            '\n', '\r', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
        ] {
            for source in [
                format!("{separator}Hello"),
                format!("Hel{separator}lo"),
                format!("Hello{separator}"),
                separator.to_string(),
            ] {
                assert!(source.parse::<PostTitle>().is_err(), "accepted {source:?}");
            }
        }
    }

    #[test]
    fn optional_post_title_distinguishes_blank_from_line_breaks() {
        assert_eq!(PostTitle::parse_optional(" \t ").unwrap(), None);
        assert_eq!(PostTitle::parse_optional("").unwrap(), None);
        assert_eq!(
            PostTitle::parse_optional(" Name ").unwrap().as_deref(),
            Some("Name")
        );
        for source in ["\n", " \r ", "\u{2028}"] {
            assert!(
                PostTitle::parse_optional(source).is_err(),
                "accepted {source:?}"
            );
        }
    }

    #[test]
    fn post_title_rejects_empty_and_whitespace_only() {
        assert!("".parse::<PostTitle>().is_err());
        assert!("   ".parse::<PostTitle>().is_err());
        assert!("\t\n".parse::<PostTitle>().is_err());
    }

    #[test]
    fn post_title_deserialize_trims_and_rejects_blank() {
        // Deserialize routes through `FromStr`, so wire input is trimmed identically
        // to in-process construction …
        assert_eq!(
            serde_json::from_str::<PostTitle>("\"  Trimmed \"").unwrap(),
            "Trimmed".parse::<PostTitle>().unwrap()
        );
        // … and a blank title is rejected on the wire rather than coerced to "".
        assert!(serde_json::from_str::<PostTitle>("\"\"").is_err());
        assert!(serde_json::from_str::<PostTitle>("\"   \"").is_err());
    }

    #[test]
    fn post_title_serializes_as_plain_string() {
        let title: PostTitle = "Title".parse().unwrap();
        assert_eq!(serde_json::to_string(&title).unwrap(), "\"Title\"");
    }

    #[test]
    fn post_title_display_exposes_inner() {
        assert_eq!(" Hi ".parse::<PostTitle>().unwrap().to_string(), "Hi");
    }
}

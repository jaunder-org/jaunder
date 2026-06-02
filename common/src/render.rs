//! Pure post-body rendering and title/metadata derivation.
//!
//! Format-driven transformation of post bodies to HTML plus extraction of
//! titles, slug seeds, and summary labels. No storage or database concerns.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// The format/markup language used to author a post body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostFormat {
    /// CommonMark/GitHub-flavored Markdown.
    Markdown,
    /// Emacs Org-mode format.
    Org,
    /// Pre-rendered HTML.
    Html,
}

/// Error returned when a string cannot be parsed as a [`PostFormat`].
#[derive(Debug, Error)]
#[error("post format must be \"markdown\", \"org\", or \"html\"")]
pub struct InvalidPostFormat;

impl fmt::Display for PostFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PostFormat::Markdown => f.write_str("markdown"),
            PostFormat::Org => f.write_str("org"),
            PostFormat::Html => f.write_str("html"),
        }
    }
}

impl FromStr for PostFormat {
    type Err = InvalidPostFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "markdown" => Ok(PostFormat::Markdown),
            "org" => Ok(PostFormat::Org),
            "html" => Ok(PostFormat::Html),
            _ => Err(InvalidPostFormat),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_format_markdown_variant() {
        let fmt = PostFormat::Markdown;
        assert_eq!(fmt, PostFormat::Markdown);
    }

    #[test]
    fn post_format_org_variant() {
        let fmt = PostFormat::Org;
        assert_eq!(fmt, PostFormat::Org);
    }

    #[test]
    fn post_format_display_round_trips() {
        assert_eq!(PostFormat::Markdown.to_string(), "markdown");
        assert_eq!(PostFormat::Org.to_string(), "org");
        assert_eq!(
            "markdown".parse::<PostFormat>().unwrap(),
            PostFormat::Markdown
        );
        assert_eq!("org".parse::<PostFormat>().unwrap(), PostFormat::Org);
    }

    #[test]
    fn post_format_rejects_invalid_value() {
        let err = "invalid".parse::<PostFormat>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "post format must be \"markdown\", \"org\", or \"html\""
        );
    }

    #[test]
    fn post_format_debug() {
        let fmt = PostFormat::Markdown;
        let debug_str = format!("{:?}", fmt);
        assert_eq!(debug_str, "Markdown");

        let fmt2 = PostFormat::Org;
        let debug_str2 = format!("{:?}", fmt2);
        assert_eq!(debug_str2, "Org");
    }

    #[test]
    fn post_format_html_roundtrips_via_display_and_from_str() {
        assert_eq!("html".parse::<PostFormat>().unwrap(), PostFormat::Html);
        assert_eq!(PostFormat::Html.to_string(), "html");
    }
}

use serde::{Deserialize, Serialize};

use crate::{media::ContentType, tag::Tag, username::Username};

/// The public representation format of a Syndication Feed.
#[macros::text_enum(
    no_serde,
    error = InvalidFeedFormat,
    message = "feed format must be \"rss\", \"atom\", or \"json\""
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[strum(serialize_all = "snake_case")]
pub enum FeedFormat {
    Rss,
    Atom,
    Json,
}

impl FeedFormat {
    #[must_use]
    pub fn ext(self) -> &'static str {
        (&self).into()
    }

    /// The media `Content-Type` served for this representation.
    ///
    /// Fixed trusted literals keep format selection and persisted representation
    /// metadata on one dual-target source of truth.
    #[must_use]
    pub fn content_type(self) -> ContentType {
        let literal = match self {
            Self::Rss => "application/rss+xml; charset=utf-8",
            Self::Atom => "application/atom+xml; charset=utf-8",
            Self::Json => "application/feed+json",
        };
        ContentType::from_trusted(literal)
    }
}

/// The public page whose Syndication Feed representation is addressed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FeedSurface {
    Site,
    SiteTag { tag: Tag },
    User { username: Username },
    UserTag { username: Username, tag: Tag },
}

/// Returns the canonical relative URL for a public Syndication Feed representation.
#[must_use]
pub fn canonicalize(surface: &FeedSurface, format: FeedFormat) -> String {
    let ext = format.ext();
    match surface {
        FeedSurface::Site => format!("/feed.{ext}"),
        FeedSurface::SiteTag { tag } => format!("/tags/{tag}/feed.{ext}"),
        FeedSurface::User { username } => format!("/~{username}/feed.{ext}"),
        FeedSurface::UserTag { username, tag } => format!("/~{username}/tags/{tag}/feed.{ext}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::parse_content_type;

    #[test]
    fn format_content_types_are_valid_and_canonical() {
        assert_eq!(
            FeedFormat::Rss.content_type(),
            parse_content_type("application/rss+xml; charset=utf-8")
        );
        assert_eq!(
            FeedFormat::Atom.content_type(),
            parse_content_type("application/atom+xml; charset=utf-8")
        );
        assert_eq!(
            FeedFormat::Json.content_type(),
            parse_content_type("application/feed+json")
        );
    }

    #[test]
    fn parses_only_lowercase_public_extensions_with_the_declared_error() {
        for (extension, format) in [
            ("rss", FeedFormat::Rss),
            ("atom", FeedFormat::Atom),
            ("json", FeedFormat::Json),
        ] {
            let parsed: FeedFormat = extension.parse().expect("public extension parses");
            assert_eq!(parsed, format);
            assert_eq!(parsed.ext(), extension);
        }

        for extension in ["", "RSS", "Atom", "JSON", "xml", "rsss"] {
            let err = extension
                .parse::<FeedFormat>()
                .expect_err("rejects non-public extension");
            assert_eq!(
                err.to_string(),
                "feed format must be \"rss\", \"atom\", or \"json\"",
                "rejects {extension:?}"
            );
        }
    }

    #[test]
    fn format_serde_representation_remains_the_variant_name() {
        for (format, wire) in [
            (FeedFormat::Rss, "\"Rss\""),
            (FeedFormat::Atom, "\"Atom\""),
            (FeedFormat::Json, "\"Json\""),
        ] {
            assert_eq!(serde_json::to_string(&format).unwrap(), wire);
            assert_eq!(serde_json::from_str::<FeedFormat>(wire).unwrap(), format);
        }
    }
}

use serde::{Deserialize, Serialize};

use crate::{media::ContentType, tag::Tag, username::Username};

/// The public representation format of a Syndication Feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FeedFormat {
    Rss,
    Atom,
    Json,
}

impl FeedFormat {
    #[must_use]
    pub fn ext(self) -> &'static str {
        match self {
            Self::Rss => "rss",
            Self::Atom => "atom",
            Self::Json => "json",
        }
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
}

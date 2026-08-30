use std::str::FromStr;

use chrono::{DateTime, Utc};
use macros::StrNewtype;
use thiserror::Error;

use common::{
    feed::FeedSurface,
    ids::PostId,
    post_summary::PostSummary,
    post_title::PostTitle,
    render::RenderedHtml,
    site::SiteTitle,
    tag::TagLabel,
    tagged_url::{CanonicalUrl, FeedUrl, HubUrl, PermalinkUrl},
};

/// Human-readable title of a public Syndication Feed document.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct FeedTitle(String);

/// Error returned for an empty [`FeedTitle`].
#[derive(Debug, Error)]
#[error("feed title cannot be empty")]
pub struct InvalidFeedTitle;

impl FromStr for FeedTitle {
    type Err = InvalidFeedTitle;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err(InvalidFeedTitle);
        }
        Ok(Self(value.to_owned()))
    }
}

impl FeedTitle {
    /// Composes the title for one Syndication Feed surface.
    #[must_use]
    pub fn for_surface(site_title: &SiteTitle, surface: &FeedSurface) -> Self {
        match surface {
            FeedSurface::Site => Self(site_title.to_string()),
            FeedSurface::SiteTag { tag } => Self(format!("{site_title} — #{tag}")),
            FeedSurface::User { username } => Self(format!("{site_title} — @{username}")),
            FeedSurface::UserTag { username, tag } => {
                Self(format!("{site_title} — @{username} #{tag}"))
            }
        }
    }
}

/// Optional nonblank descriptive text for a public Syndication Feed.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct FeedDescription(String);

/// Error returned for an empty [`FeedDescription`].
#[derive(Debug, Error)]
#[error("feed description cannot be empty")]
pub struct InvalidFeedDescription;

impl FromStr for FeedDescription {
    type Err = InvalidFeedDescription;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err(InvalidFeedDescription);
        }
        Ok(Self(value.to_owned()))
    }
}

/// Feed-level metadata: what a rendered feed document says about itself.
///
/// `canonical_url` (where the feed's subject lives) and `self_url` (where the feed
/// document itself lives) carry distinct roles, so transposing them is a compile error
/// rather than a feed that points at itself as its own subject (#875):
///
/// ```compile_fail
/// # use host::feed::metadata::FeedMetadata;
/// # fn f(a: FeedMetadata, b: FeedMetadata) -> FeedMetadata {
/// FeedMetadata { canonical_url: b.self_url, self_url: b.canonical_url, ..a }
/// # }
/// ```
///
/// The correct assignment compiles — same fixture, so the negative above can only be
/// failing for the transposition:
///
/// ```
/// # use host::feed::metadata::FeedMetadata;
/// # fn f(a: FeedMetadata, b: FeedMetadata) -> FeedMetadata {
/// FeedMetadata { canonical_url: b.canonical_url, self_url: b.self_url, ..a }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct FeedMetadata {
    pub title: FeedTitle,
    pub description: Option<FeedDescription>,
    pub canonical_url: CanonicalUrl,
    pub self_url: FeedUrl,
    pub hub_url: Option<HubUrl>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct FeedItem {
    pub id: PostId, // last_post_id input to ETag
    pub title: Option<PostTitle>,
    pub permalink: PermalinkUrl,
    pub summary: Option<PostSummary>,
    pub content_html: RenderedHtml,
    pub published_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<TagLabel>,
}

impl crate::feed::window::HasPublishedAt for FeedItem {
    fn published_at(&self) -> DateTime<Utc> {
        self.published_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common::feed::FeedSurface;
    use common::{
        site::SiteTitle,
        test_support::{parse_post_title, parse_url},
    };

    #[test]
    fn feed_title_parses_trims_and_rejects_blank() {
        assert_eq!("  A Feed  ".parse::<FeedTitle>().unwrap(), "A Feed");
        assert!("".parse::<FeedTitle>().is_err());
        assert!("   ".parse::<FeedTitle>().is_err());
    }

    #[test]
    fn feed_description_parses_trims_and_rejects_blank() {
        assert_eq!(
            "  Posts from Alice  ".parse::<FeedDescription>().unwrap(),
            "Posts from Alice"
        );
        assert!("".parse::<FeedDescription>().is_err());
        assert!("\t\n".parse::<FeedDescription>().is_err());
    }

    #[test]
    fn feed_title_composes_every_surface() {
        let site = "Jaunder".parse::<SiteTitle>().unwrap();
        assert_eq!(FeedTitle::for_surface(&site, &FeedSurface::Site), "Jaunder");
        assert_eq!(
            FeedTitle::for_surface(
                &site,
                &FeedSurface::SiteTag {
                    tag: "rust".parse().unwrap(),
                },
            ),
            "Jaunder — #rust"
        );
        assert_eq!(
            FeedTitle::for_surface(
                &site,
                &FeedSurface::User {
                    username: "alice".parse().unwrap(),
                },
            ),
            "Jaunder — @alice"
        );
        assert_eq!(
            FeedTitle::for_surface(
                &site,
                &FeedSurface::UserTag {
                    username: "alice".parse().unwrap(),
                    tag: "rust".parse().unwrap(),
                },
            ),
            "Jaunder — @alice #rust"
        );
    }

    fn item(id: PostId, ts: DateTime<Utc>) -> FeedItem {
        FeedItem {
            id,
            title: Some(parse_post_title("t")),
            permalink: parse_url("https://ex.com/p"),
            summary: None,
            content_html: common::test_support::rendered_html("<p>c</p>"),
            published_at: ts,
            updated_at: ts,
            tags: vec![],
        }
    }

    #[test]
    fn feed_item_implements_has_published_at() {
        use crate::feed::window::{HasPublishedAt, HybridWindow};
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let i = item(PostId::from(1), now);
        assert_eq!(<FeedItem as HasPublishedAt>::published_at(&i), now);
        // And exercise it through HybridWindow::select to confirm trait wiring.
        let items = [item(PostId::from(1), now)];
        let kept = HybridWindow::default().select(&items, now);
        assert_eq!(kept.len(), 1);
    }
}

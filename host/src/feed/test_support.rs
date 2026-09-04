//! Typed fixtures shared by host Syndication Feed renderer tests.

use chrono::{DateTime, Utc};
use common::{
    feed::FeedSurface,
    ids::PostId,
    render::RenderedHtml,
    tagged_url::{FeedUrl, PermalinkUrl},
    test_support::{parse_site_title, parse_url, parse_utc_instant},
};

use super::{FeedItem, FeedMetadata, FeedTitle};

/// Builds valid baseline metadata for one format-specific Syndication Feed URL.
///
/// Format tests override optional description and `WebSub` Hub fields with struct update.
#[must_use]
pub fn feed_metadata(self_url: FeedUrl) -> FeedMetadata {
    FeedMetadata {
        title: FeedTitle::for_surface(&parse_site_title("Site"), &FeedSurface::Site),
        description: None,
        canonical_url: parse_url("https://example.com/"),
        self_url,
        hub_url: None,
        representation_modified_at: parse_utc_instant("2026-01-01T00:00:00Z").into(),
    }
}

/// Builds a Syndication Feed item with shared absent optional fields.
///
/// Format tests override title, summary, and tags with struct update.
#[must_use]
pub fn feed_item(
    id: PostId,
    permalink: PermalinkUrl,
    content_html: RenderedHtml,
    timestamp: DateTime<Utc>,
) -> FeedItem {
    FeedItem {
        id,
        title: None,
        permalink,
        summary: None,
        content_html,
        published_at: timestamp,
        updated_at: timestamp,
        tags: vec![],
    }
}

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::etag::ETag;
use crate::ids::PostId;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::render::RenderedHtml;
use crate::tag::TagLabel;
use crate::tagged_url::{CanonicalUrl, FeedUrl, HubUrl, PermalinkUrl};

/// Feed-level metadata: what a rendered feed document says about itself.
///
/// `canonical_url` (where the feed's subject lives) and `self_url` (where the feed
/// document itself lives) carry distinct roles, so transposing them is a compile error
/// rather than a feed that points at itself as its own subject (#875):
///
/// ```compile_fail
/// # use common::feed::metadata::FeedMetadata;
/// # fn f(a: FeedMetadata, b: FeedMetadata) -> FeedMetadata {
/// FeedMetadata { canonical_url: b.self_url, self_url: b.canonical_url, ..a }
/// # }
/// ```
///
/// The correct assignment compiles — same fixture, so the negative above can only be
/// failing for the transposition:
///
/// ```
/// # use common::feed::metadata::FeedMetadata;
/// # fn f(a: FeedMetadata, b: FeedMetadata) -> FeedMetadata {
/// FeedMetadata { canonical_url: b.canonical_url, self_url: b.self_url, ..a }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct FeedMetadata {
    pub title: String,
    pub description: Option<String>,
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

/// Strong validator over the feed's identity fields (max `updated_at`, item count, last
/// post id) — a `"sha256-<64hex>"` [`ETag`]. The `ETag` door owns the digest→hex→prefix→
/// quotes format; this fn owns only *which bytes* identify a feed version.
#[must_use]
pub fn feed_etag(items: &[FeedItem], generated_at: DateTime<Utc>) -> ETag {
    let mut hasher = Sha256::new();
    let max_updated = items
        .iter()
        .map(|i| i.updated_at)
        .max()
        .unwrap_or(generated_at);
    let last_id = items.last().map_or(0, |i| i64::from(i.id));
    hasher.update(max_updated.to_rfc3339().as_bytes());
    hasher.update(b"|");
    hasher.update((items.len() as u64).to_le_bytes());
    hasher.update(b"|");
    hasher.update(last_id.to_le_bytes());
    ETag::from_sha256(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{parse_post_title, parse_url};
    use chrono::TimeZone;

    fn item(id: PostId, ts: DateTime<Utc>) -> FeedItem {
        FeedItem {
            id,
            title: Some(parse_post_title("t")),
            permalink: parse_url("https://ex.com/p"),
            summary: None,
            content_html: RenderedHtml::from_trusted("<p>c</p>"),
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

    #[test]
    fn etag_stable_for_identical_input() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let items = vec![item(PostId::from(1), now), item(PostId::from(2), now)];
        assert_eq!(feed_etag(&items, now), feed_etag(&items, now));
    }

    #[test]
    fn etag_changes_when_count_changes() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let a = vec![item(PostId::from(1), now)];
        let b = vec![item(PostId::from(1), now), item(PostId::from(2), now)];
        assert_ne!(feed_etag(&a, now), feed_etag(&b, now));
    }

    #[test]
    fn etag_for_empty_uses_generated_at() {
        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
        assert_ne!(feed_etag(&[], t1), feed_etag(&[], t2));
    }

    #[test]
    fn etag_changes_when_updated_at_changes() {
        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
        let a = vec![item(PostId::from(1), t1)];
        let b = vec![item(PostId::from(1), t2)];
        assert_ne!(feed_etag(&a, t1), feed_etag(&b, t1));
    }
}

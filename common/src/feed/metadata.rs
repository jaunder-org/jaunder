use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::ids::PostId;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::render::RenderedHtml;
use crate::tag::TagLabel;

#[derive(Debug, Clone)]
pub struct FeedMetadata {
    pub title: String,
    pub description: Option<String>,
    pub canonical_url: String,
    pub self_url: String,
    pub hub_url: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct FeedItem {
    pub id: PostId, // last_post_id input to ETag
    pub title: Option<PostTitle>,
    pub permalink: String,
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

/// Strong validator. Format: `"sha256-<hex32>"`.
#[must_use]
pub fn feed_etag(items: &[FeedItem], generated_at: DateTime<Utc>) -> String {
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
    let digest = hasher.finalize();
    let hex = digest.iter().take(16).fold(String::new(), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    });
    format!("\"sha256-{hex}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::parse_post_title;
    use chrono::TimeZone;

    fn item(id: PostId, ts: DateTime<Utc>) -> FeedItem {
        FeedItem {
            id,
            title: Some(parse_post_title("t")),
            permalink: "/p".into(),
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

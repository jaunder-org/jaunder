use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

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
    pub id: i64, // last_post_id input to ETag
    pub title: Option<String>,
    pub permalink: String,
    pub summary: Option<String>,
    pub content_html: String,
    pub published_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

impl crate::feed::window::HasPublishedAt for FeedItem {
    fn published_at(&self) -> DateTime<Utc> {
        self.published_at
    }
}

/// Strong validator. Format: `"sha256-<hex32>"`.
pub fn feed_etag(items: &[FeedItem], generated_at: DateTime<Utc>) -> String {
    let mut hasher = Sha256::new();
    let max_updated = items
        .iter()
        .map(|i| i.updated_at)
        .max()
        .unwrap_or(generated_at);
    let last_id = items.last().map(|i| i.id).unwrap_or(0);
    hasher.update(max_updated.to_rfc3339().as_bytes());
    hasher.update(b"|");
    hasher.update((items.len() as u64).to_le_bytes());
    hasher.update(b"|");
    hasher.update(last_id.to_le_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("\"sha256-{hex}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn item(id: i64, ts: DateTime<Utc>) -> FeedItem {
        FeedItem {
            id,
            title: Some("t".into()),
            permalink: "/p".into(),
            summary: None,
            content_html: "<p>c</p>".into(),
            published_at: ts,
            updated_at: ts,
            tags: vec![],
        }
    }

    #[test]
    fn etag_stable_for_identical_input() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let items = vec![item(1, now), item(2, now)];
        assert_eq!(feed_etag(&items, now), feed_etag(&items, now));
    }

    #[test]
    fn etag_changes_when_count_changes() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let a = vec![item(1, now)];
        let b = vec![item(1, now), item(2, now)];
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
        let a = vec![item(1, t1)];
        let b = vec![item(1, t2)];
        assert_ne!(feed_etag(&a, t1), feed_etag(&b, t1));
    }
}

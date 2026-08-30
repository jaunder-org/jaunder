//! Host-side construction of HTTP `ETag` values.

use std::str::FromStr;

use crate::feed::FeedItem;
use chrono::{DateTime, Utc};
use common::{
    etag::ETag, media::ContentHash, post_body::PostBody, post_summary::PostSummary,
    post_title::PostTitle, render::PostFormat, tag::TagLabel,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Computes the `ETag` of `bytes`: `"sha256-<hex>"` over `Sha256::digest(bytes)`.
#[must_use]
pub fn sha256_of(bytes: impl AsRef<[u8]>) -> ETag {
    from_sha256(Sha256::digest(bytes.as_ref()).into())
}

/// Constructs the `ETag` of a precomputed 32-byte SHA-256 digest.
#[must_use]
pub fn from_sha256(digest: [u8; 32]) -> ETag {
    from_content_hash(&ContentHash::from_digest(digest))
}

/// Constructs the `ETag` of an already-validated content hash.
///
/// # Panics
///
/// Never for a valid [`ContentHash`]: its invariant guarantees this function's
/// canonical strong quoted tag satisfies `ETag` parsing.
#[must_use]
pub fn from_content_hash(hash: &ContentHash) -> ETag {
    let hex = hash.as_ref();
    let mut value = String::with_capacity(hex.len() + "\"sha256-\"".len());
    value.push('"');
    value.push_str("sha256-");
    value.push_str(hex);
    value.push('"');
    match ETag::from_str(&value) {
        Ok(etag) => etag,
        Err(_) => unreachable!("canonical SHA-256 ETag is valid"),
    }
}

/// Computes the strong validator for a Syndication Feed's identity fields.
#[must_use]
pub fn feed_etag(items: &[FeedItem], generated_at: DateTime<Utc>) -> ETag {
    let mut hasher = Sha256::new();
    let max_updated = items
        .iter()
        .map(|item| item.updated_at)
        .max()
        .unwrap_or(generated_at);
    let last_id = items.last().map_or(0, |item| i64::from(item.id));
    hasher.update(max_updated.to_rfc3339().as_bytes());
    hasher.update(b"|");
    hasher.update((items.len() as u64).to_le_bytes());
    hasher.update(b"|");
    hasher.update(last_id.to_le_bytes());
    from_sha256(hasher.finalize().into())
}

/// Computes the canonical strong validator for a Post's mutable content.
#[must_use]
pub fn post_content_etag<'a>(
    title: Option<&'a PostTitle>,
    body: &'a PostBody,
    format: &'a PostFormat,
    summary: Option<&'a PostSummary>,
    tags: impl IntoIterator<Item = &'a TagLabel>,
    draft: bool,
) -> ETag {
    #[derive(Serialize)]
    struct Content<'a> {
        title: Option<&'a PostTitle>,
        body: &'a PostBody,
        format: String,
        summary: Option<&'a PostSummary>,
        tags: Vec<&'a TagLabel>,
        draft: bool,
    }

    let content = Content {
        title,
        body,
        format: format.to_string(),
        summary,
        tags: tags.into_iter().collect(),
        draft,
    };
    let bytes = serde_json::to_vec(&content).unwrap_or_else(|_| Vec::new());
    sha256_of(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use common::ids::PostId;
    use common::test_support::{parse_content_hash, parse_post_body, parse_post_title, parse_url};

    const HASH64: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn constructors_produce_canonical_sha256_etags() {
        assert_eq!(
            from_sha256([0; 32]),
            format!("\"sha256-{}\"", "0".repeat(64)).as_str()
        );
        assert_eq!(sha256_of(b""), format!("\"sha256-{HASH64}\"").as_str());
        let hash = parse_content_hash(HASH64);
        assert_eq!(
            from_content_hash(&hash),
            format!("\"sha256-{HASH64}\"").as_str()
        );
    }

    #[test]
    fn post_content_etag_preserves_historical_projection() {
        let title = parse_post_title("A title");
        let body = parse_post_body("A body\n");
        assert_eq!(
            post_content_etag(Some(&title), &body, &PostFormat::Markdown, None, [], false),
            "\"sha256-499c5c7ff8a46045dd66cecf11911c2136fc93f247af622f6eec5c93efad7388\""
        );
    }
    fn feed_item(id: PostId, timestamp: DateTime<Utc>) -> FeedItem {
        FeedItem {
            id,
            title: Some(parse_post_title("t")),
            permalink: parse_url("https://ex.com/p"),
            summary: None,
            content_html: common::test_support::rendered_html("<p>c</p>"),
            published_at: timestamp,
            updated_at: timestamp,
            tags: vec![],
        }
    }

    #[test]
    fn feed_etag_is_stable_and_tracks_its_identity_inputs() {
        let first = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let second = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
        let one = vec![feed_item(PostId::from(1), first)];
        let two = vec![
            feed_item(PostId::from(1), first),
            feed_item(PostId::from(2), first),
        ];

        assert_eq!(feed_etag(&one, first), feed_etag(&one, first));
        assert_ne!(feed_etag(&one, first), feed_etag(&two, first));
        assert_ne!(feed_etag(&[], first), feed_etag(&[], second));
        assert_ne!(
            feed_etag(&one, first),
            feed_etag(&[feed_item(PostId::from(1), second)], first)
        );
    }
}

//! Host-side construction of HTTP `ETag` values.

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use common::{
    etag::ETag, feed::FeedFormat, media::ContentHash, post_body::PostBody,
    post_summary::PostSummary, post_title::PostTitle, render::PostFormat, tag::TagLabel,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::feed::{FeedItem, FeedMetadata};

/// RSS serializer wire-layout revision. Increment when RSS bytes can change.
pub const RSS_SERIALIZER_REVISION: u16 = 1;
/// Atom serializer wire-layout revision. Increment when Atom bytes can change.
pub const ATOM_SERIALIZER_REVISION: u16 = 1;
/// JSON Feed serializer wire-layout revision. Increment when JSON Feed bytes can change.
pub const JSON_SERIALIZER_REVISION: u16 = 1;

/// A validated, persisted digest of a Syndication Feed's semantic serializer inputs.
///
/// This intentionally exposes no constituent fields: storage may compare and persist the
/// canonical digest, while only this module defines the complete identity tuple.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FeedSemanticFingerprint(ContentHash);

/// Error returned for a noncanonical persisted feed semantic fingerprint.
#[derive(Debug, Error)]
#[error("feed semantic fingerprint must be 64 lowercase hex characters ([0-9a-f]{{64}})")]
pub struct InvalidFeedSemanticFingerprint;

impl FeedSemanticFingerprint {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl fmt::Display for FeedSemanticFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for FeedSemanticFingerprint {
    type Err = InvalidFeedSemanticFingerprint;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value
            .parse()
            .map(Self)
            .map_err(|_| InvalidFeedSemanticFingerprint)
    }
}

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

/// Computes the closed semantic identity used for cache replacement.
#[must_use]
pub fn feed_semantic_fingerprint(
    format: FeedFormat,
    metadata: &FeedMetadata,
    items: &[FeedItem],
) -> FeedSemanticFingerprint {
    semantic_fingerprint_with_revision(format, serializer_revision(format), metadata, items)
}

/// Computes a strong validator from a complete semantic identity and the selected
/// representation time.
///
/// `representation_modified_at` is deliberately absent from
/// [`feed_semantic_fingerprint`]: storage selects it after deciding whether that semantic
/// identity changed.
#[must_use]
pub fn feed_etag(
    fingerprint: &FeedSemanticFingerprint,
    representation_modified_at: DateTime<Utc>,
) -> ETag {
    let mut hasher = Sha256::new();
    write_bytes(&mut hasher, b"jaunder.feed.etag.v1");
    write_bytes(&mut hasher, fingerprint.as_str().as_bytes());
    write_timestamp(&mut hasher, representation_modified_at);
    from_sha256(hasher.finalize().into())
}

fn semantic_fingerprint_with_revision(
    format: FeedFormat,
    revision: u16,
    metadata: &FeedMetadata,
    items: &[FeedItem],
) -> FeedSemanticFingerprint {
    let mut hasher = Sha256::new();
    write_bytes(&mut hasher, b"jaunder.feed.semantic-identity.v1");
    write_bytes(
        &mut hasher,
        match format {
            FeedFormat::Rss => b"rss",
            FeedFormat::Atom => b"atom",
            FeedFormat::Json => b"json",
        },
    );
    write_bytes(&mut hasher, &revision.to_be_bytes());
    write_string(&mut hasher, metadata.title.as_ref());
    write_optional_string(
        &mut hasher,
        metadata.description.as_ref().map(AsRef::as_ref),
    );
    write_string(&mut hasher, metadata.canonical_url.as_ref());
    write_string(&mut hasher, metadata.self_url.as_ref());
    write_optional_string(&mut hasher, metadata.hub_url.as_ref().map(AsRef::as_ref));
    write_bytes(&mut hasher, &(items.len() as u64).to_be_bytes());
    for item in items {
        write_bytes(&mut hasher, &i64::from(item.id).to_be_bytes());
        write_optional_string(&mut hasher, item.title.as_ref().map(AsRef::as_ref));
        write_string(&mut hasher, item.permalink.as_ref());
        write_optional_string(&mut hasher, item.summary.as_ref().map(AsRef::as_ref));
        write_string(&mut hasher, item.content_html.as_ref());
        write_timestamp(&mut hasher, item.published_at);
        write_timestamp(&mut hasher, item.updated_at);
        write_bytes(&mut hasher, &(item.tags.len() as u64).to_be_bytes());
        for tag in &item.tags {
            write_string(&mut hasher, tag.as_ref());
        }
    }
    FeedSemanticFingerprint(ContentHash::from_digest(hasher.finalize().into()))
}

fn write_optional_string(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            write_string(hasher, value);
        }
        None => hasher.update([0]),
    }
}

fn write_string(hasher: &mut Sha256, value: &str) {
    write_bytes(hasher, value.as_bytes());
}

fn write_timestamp(hasher: &mut Sha256, value: DateTime<Utc>) {
    hasher.update(value.timestamp().to_be_bytes());
    hasher.update(value.timestamp_subsec_nanos().to_be_bytes());
}

fn write_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

const fn serializer_revision(format: FeedFormat) -> u16 {
    match format {
        FeedFormat::Rss => RSS_SERIALIZER_REVISION,
        FeedFormat::Atom => ATOM_SERIALIZER_REVISION,
        FeedFormat::Json => JSON_SERIALIZER_REVISION,
    }
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
    use crate::feed::{FeedDescription, FeedTitle};
    use chrono::TimeZone;
    use common::{
        ids::PostId,
        test_support::{parse_post_summary, parse_post_title, parse_url, rendered_html},
    };

    fn time(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, day, 1, 2, 3).unwrap()
    }
    fn metadata() -> FeedMetadata {
        FeedMetadata {
            title: "Feed".parse::<FeedTitle>().unwrap(),
            description: Some("Description".parse::<FeedDescription>().unwrap()),
            canonical_url: parse_url("https://example.com/"),
            self_url: parse_url("https://example.com/feed.atom"),
            hub_url: Some(parse_url("https://hub.example.com/")),
            representation_modified_at: time(1),
        }
    }
    fn item(id: i64) -> FeedItem {
        FeedItem {
            id: PostId::from(id),
            title: Some(parse_post_title("Title")),
            permalink: parse_url(format!("https://example.com/{id}").as_str()),
            summary: Some(parse_post_summary("Summary")),
            content_html: rendered_html("<p>Content</p>"),
            published_at: time(1),
            updated_at: time(2),
            tags: vec!["one".parse().unwrap(), "two".parse().unwrap()],
        }
    }
    fn fingerprint(metadata: &FeedMetadata, items: &[FeedItem]) -> FeedSemanticFingerprint {
        feed_semantic_fingerprint(FeedFormat::Atom, metadata, items)
    }

    #[test]
    fn constructors_produce_canonical_sha256_etags() {
        assert_eq!(
            from_sha256([0; 32]),
            format!("\"sha256-{}\"", "0".repeat(64)).as_str()
        );
    }

    #[test]
    fn fingerprint_validates_its_persistent_representation() {
        let value = fingerprint(&metadata(), &[item(1)]);
        assert_eq!(
            value
                .to_string()
                .parse::<FeedSemanticFingerprint>()
                .unwrap(),
            value
        );
        assert!("A".repeat(64).parse::<FeedSemanticFingerprint>().is_err());
    }

    fn etag(
        format: FeedFormat,
        metadata: &FeedMetadata,
        items: &[FeedItem],
        representation_modified_at: DateTime<Utc>,
    ) -> ETag {
        feed_etag(
            &feed_semantic_fingerprint(format, metadata, items),
            representation_modified_at,
        )
    }

    #[test]
    fn feed_etag_covers_complete_inputs_for_each_format() {
        let metadata = metadata();
        let items = vec![item(1), item(2)];
        let formats = [FeedFormat::Rss, FeedFormat::Atom, FeedFormat::Json];
        let metadata_mutations: [fn(&mut FeedMetadata); 5] = [
            |metadata| metadata.title = "Other".parse().unwrap(),
            |metadata| metadata.description = None,
            |metadata| metadata.canonical_url = parse_url("https://other.example/"),
            |metadata| metadata.self_url = parse_url("https://example.com/other.atom"),
            |metadata| metadata.hub_url = None,
        ];
        let item_mutations: [fn(&mut FeedItem); 8] = [
            |item| item.id = PostId::from(9),
            |item| item.title = None,
            |item| item.permalink = parse_url("https://example.com/other"),
            |item| item.summary = None,
            |item| item.content_html = rendered_html("<p>Other</p>"),
            |item| item.published_at = time(3),
            |item| item.updated_at = time(3),
            |item| item.tags.reverse(),
        ];

        for format in formats {
            let baseline = etag(format, &metadata, &items, time(1));
            assert_eq!(
                baseline,
                etag(format, &metadata, &items, time(1)),
                "{format:?} ETag is stable for identical complete inputs",
            );
            for mutate in metadata_mutations {
                let mut changed = metadata.clone();
                mutate(&mut changed);
                assert_ne!(
                    baseline,
                    etag(format, &changed, &items, time(1)),
                    "{format:?} ETag changes for metadata",
                );
            }
            for mutate in item_mutations {
                let mut changed = items.clone();
                mutate(&mut changed[0]);
                assert_ne!(
                    baseline,
                    etag(format, &metadata, &changed, time(1)),
                    "{format:?} ETag changes for item data",
                );
            }
            let mut reordered = items.clone();
            reordered.reverse();
            assert_ne!(
                baseline,
                etag(format, &metadata, &reordered, time(1)),
                "{format:?} ETag changes for item order",
            );
            assert_ne!(
                baseline,
                etag(format, &metadata, &items, time(2)),
                "{format:?} ETag changes for representation modification time",
            );
            assert_ne!(
                baseline,
                feed_etag(
                    &semantic_fingerprint_with_revision(
                        format,
                        serializer_revision(format) + 1,
                        &metadata,
                        &items,
                    ),
                    time(1),
                ),
                "{format:?} ETag changes for its serializer revision",
            );
        }

        assert_ne!(
            etag(FeedFormat::Rss, &metadata, &items, time(1)),
            etag(FeedFormat::Atom, &metadata, &items, time(1)),
        );
        assert_ne!(
            etag(FeedFormat::Atom, &metadata, &items, time(1)),
            etag(FeedFormat::Json, &metadata, &items, time(1)),
        );
    }

    #[test]
    fn semantic_fingerprint_excludes_representation_time() {
        let metadata = metadata();
        let mut changed = metadata.clone();
        changed.representation_modified_at = time(2);
        let items = vec![item(1)];

        assert_eq!(
            feed_semantic_fingerprint(FeedFormat::Atom, &metadata, &items),
            feed_semantic_fingerprint(FeedFormat::Atom, &changed, &items),
        );
    }
}

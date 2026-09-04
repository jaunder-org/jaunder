//! Feed-cache fixture builder that keeps stored representation metadata derived from its path.

use super::confirmed_for;
use crate::AppState;
use crate::feed_cache::FeedCacheRow;

use chrono::Timelike;
use common::{etag::ETag, feed::FeedFormat, test_support::parse_etag, time::UtcInstant};
use host::feed::{FeedPath, SyndicationFeedRepresentation};
use std::sync::Arc;

/// A coherent cached feed row whose representation metadata is derived from its
/// typed path.
pub struct SeedFeedCache {
    feed_path: FeedPath,
    format: FeedFormat,
    body: String,
    etag: ETag,
    updated_at: UtcInstant,
    generated_at: UtcInstant,
}

impl SeedFeedCache {
    /// Create a valid cached feed for `feed_path`, using one instant for both
    /// cache timestamp roles.
    ///
    /// # Panics
    ///
    /// Panics if `feed_path` does not identify a recognized feed surface and format.
    #[must_use]
    pub fn new(feed_path: FeedPath) -> Self {
        let format = feed_path
            .parts()
            .expect("validated feed path has a recoverable format")
            .1;
        let now = storage_instant(UtcInstant::now());
        Self {
            feed_path,
            format,
            body: default_body(format),
            etag: parse_etag("\"sha256-seeded-feed\""),
            updated_at: now,
            generated_at: now,
        }
    }

    /// Override the serialized feed body.
    #[must_use]
    pub fn body(mut self, body: String) -> Self {
        self.body = body;
        self
    }

    /// Override the stored entity tag.
    #[must_use]
    pub fn etag(mut self, etag: ETag) -> Self {
        self.etag = etag;
        self
    }

    /// Override the cache update timestamp.
    #[must_use]
    pub fn updated_at(mut self, updated_at: UtcInstant) -> Self {
        self.updated_at = storage_instant(updated_at);
        self
    }

    /// Override the feed generation timestamp.
    #[must_use]
    pub fn generated_at(mut self, generated_at: UtcInstant) -> Self {
        self.generated_at = storage_instant(generated_at);
        self
    }

    /// Build the coherent cache row without writing it.
    ///
    /// # Panics
    ///
    /// If the path-derived representation metadata cannot form a coherent
    /// cache row, which would violate the fixture's construction invariant.
    #[must_use]
    pub fn build(self) -> FeedCacheRow {
        let format = self.format;
        let representation = SyndicationFeedRepresentation::try_from_stored(
            format,
            format.content_type(),
            self.body,
        )
        .expect("path-derived feed representation has matching metadata");
        FeedCacheRow::new(
            self.feed_path,
            representation,
            self.etag,
            self.updated_at,
            self.generated_at,
        )
        .expect("path-derived feed representation matches feed cache path")
    }

    /// Persist one cache row through the state-owned write scope.
    ///
    /// # Panics
    ///
    /// If the fixture write fails or does not receive a confirmed commit.
    pub async fn seed(self, state: &AppState) -> FeedCacheRow {
        let row = self.build();
        let returned = row.clone();
        let feed_cache = Arc::clone(&state.feed_cache);
        let outcome = state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { feed_cache.upsert(transaction, row).await })
            })
            .await
            .expect("seed feed cache should be created");
        confirmed_for(outcome, "seed feed cache");
        returned
    }
}
fn storage_instant(instant: UtcInstant) -> UtcInstant {
    let instant = instant.value();
    UtcInstant::from(
        instant
            .with_nanosecond(instant.nanosecond() / 1_000 * 1_000)
            .expect("truncated nanoseconds are valid"),
    )
}

fn default_body(format: common::feed::FeedFormat) -> String {
    match format {
        common::feed::FeedFormat::Rss => concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<rss version=\"2.0\"><channel><title>Seeded feed</title>",
            "<link>https://example.test/feed.rss</link>",
            "<description>Seeded feed</description></channel></rss>"
        )
        .to_owned(),
        common::feed::FeedFormat::Atom => concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>Seeded feed</title>",
            "<id>https://example.test/feed.atom</id>",
            "<updated>2026-01-01T00:00:00Z</updated></feed>"
        )
        .to_owned(),
        common::feed::FeedFormat::Json => {
            r#"{"version":"https://jsonfeed.org/version/1.1","title":"Seeded feed","items":[]}"#
                .to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SeedFeedCache;
    use crate::test_support::{Backend, backends, fp};

    use common::{
        feed::FeedFormat,
        test_support::{parse_etag, parse_utc_instant},
    };
    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn build_derives_rss_representation_from_path() {
        let row = SeedFeedCache::new(fp("/feed.rss")).build();

        assert_eq!(row.feed_path(), "/feed.rss");
        assert_eq!(row.representation().format(), FeedFormat::Rss);
        assert_eq!(
            row.representation().content_type(),
            "application/rss+xml; charset=utf-8"
        );
        assert!(
            row.representation().body().contains("<rss"),
            "default RSS body must be a meaningful RSS document"
        );
        assert!(
            row.representation().body().contains("<channel>"),
            "default RSS body must include its required channel"
        );
    }

    #[test]
    fn build_derives_atom_representation_from_path() {
        let row = SeedFeedCache::new(fp("/feed.atom")).build();

        assert_eq!(row.feed_path(), "/feed.atom");
        assert_eq!(row.representation().format(), FeedFormat::Atom);
        assert_eq!(
            row.representation().content_type(),
            "application/atom+xml; charset=utf-8"
        );
        assert!(
            row.representation().body().contains("<feed"),
            "default Atom body must be a meaningful Atom document"
        );
        assert!(
            row.representation().body().contains("<title>"),
            "default Atom body must include its required title"
        );
    }

    #[test]
    fn build_derives_json_representation_from_path() {
        let row = SeedFeedCache::new(fp("/feed.json")).build();
        let body: serde_json::Value = serde_json::from_str(row.representation().body())
            .expect("default JSON body must be valid JSON");

        assert_eq!(row.feed_path(), "/feed.json");
        assert_eq!(row.representation().format(), FeedFormat::Json);
        assert_eq!(row.representation().content_type(), "application/feed+json");
        assert_eq!(body["version"], "https://jsonfeed.org/version/1.1");
        assert!(
            body["items"].is_array(),
            "default JSON body must be a JSON Feed document"
        );
    }

    #[test]
    fn build_defaults_timestamp_roles_to_one_instant() {
        let row = SeedFeedCache::new(fp("/feed.rss")).build();

        assert_eq!(row.updated_at, row.generated_at);
    }

    #[test]
    fn build_preserves_each_override() {
        let updated_at = parse_utc_instant("2026-08-25T01:02:03.123456Z");
        let generated_at = parse_utc_instant("2026-08-25T01:02:04.123456Z");
        let row = SeedFeedCache::new(fp("/feed.rss"))
            .body("<rss><channel><title>Overridden</title></channel></rss>".to_owned())
            .etag(parse_etag("\"sha256-overridden\""))
            .updated_at(updated_at)
            .generated_at(generated_at)
            .build();

        assert_eq!(
            row.representation().body(),
            "<rss><channel><title>Overridden</title></channel></rss>"
        );
        assert_eq!(row.etag, parse_etag("\"sha256-overridden\""));
        assert_eq!(row.updated_at, updated_at);
        assert_eq!(row.generated_at, generated_at);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn build_does_not_persist_a_feed_cache_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        let feed_path = fp("/feed.rss");

        let row = SeedFeedCache::new(feed_path.clone()).build();

        assert_eq!(row.feed_path(), &feed_path);
        assert!(
            env.state
                .feed_cache
                .get(&feed_path)
                .await
                .unwrap()
                .is_none(),
            "build must not write to storage"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn seed_persists_and_returns_the_inserted_feed_cache_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        let feed_path = fp("/feed.atom");
        let persisted_at = parse_utc_instant("2026-08-25T01:02:03.123456789Z");
        let stored_at = parse_utc_instant("2026-08-25T01:02:03.123456Z");

        let seeded = SeedFeedCache::new(feed_path.clone())
            .updated_at(persisted_at)
            .generated_at(persisted_at)
            .seed(&env.state)
            .await;
        let stored = env
            .state
            .feed_cache
            .get(&feed_path)
            .await
            .unwrap()
            .expect("seeded feed cache row exists");

        assert_eq!(seeded.updated_at, stored_at);
        assert_eq!(seeded.generated_at, stored_at);
        assert_eq!(seeded, stored);
    }
}

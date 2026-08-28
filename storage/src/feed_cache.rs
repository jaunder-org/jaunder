//! Cached, fully-rendered feed bodies keyed by their canonical (decoded) path
//! form. The cache layer is the single source of truth for what bytes get
//! served by `GET /feed.{rss,atom,json}` and the other feed endpoints.

use async_trait::async_trait;
use common::{etag::ETag, feed::FeedFormat, media::ContentType, time::UtcInstant};
use host::feed::{
    FeedPath, MismatchedStoredSyndicationFeedMetadata, SyndicationFeedRepresentation,
};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;
use crate::role_instant::impl_role_instant;

/// The `feed_cache.updated_at` storage timestamp role, distinct from
/// `generated_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct FeedCacheUpdatedAt(UtcInstant);
impl_role_instant!(FeedCacheUpdatedAt, UtcInstant);

/// The `feed_cache.generated_at` storage timestamp role, distinct from
/// `updated_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct FeedCacheGeneratedAt(UtcInstant);
impl_role_instant!(FeedCacheGeneratedAt, UtcInstant);
/// A cached rendered Syndication Feed whose path and representation formats agree.
///
/// The constructor couples the path's format with the closed representation (#697;
/// ADR-0063), preventing the independently forgeable primitive fields this replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedCacheRow {
    feed_path: FeedPath,
    representation: SyndicationFeedRepresentation,
    /// The stored strong `ETag`. Decodes through the `ETag` sqlx bridge (#438/#634), so a
    /// corrupt/migrated value is rejected as a `ColumnDecode` error on read-back.
    pub etag: ETag,
    pub updated_at: UtcInstant,
    pub generated_at: UtcInstant,
}

/// A feed-cache path whose format conflicts with its rendered representation.
#[derive(Debug, Error)]
#[error(
    "feed cache path {feed_path} has format {path_format:?}, which conflicts with representation format {representation_format:?}"
)]
pub struct MismatchedFeedCacheRowFormat {
    feed_path: FeedPath,
    path_format: Option<FeedFormat>,
    representation_format: FeedFormat,
}

impl FeedCacheRow {
    /// Couples a cache key to a rendered Syndication Feed representation.
    ///
    /// # Errors
    ///
    /// Returns [`MismatchedFeedCacheRowFormat`] when the path's requested format
    /// differs from the representation's format.
    pub fn new(
        feed_path: FeedPath,
        representation: SyndicationFeedRepresentation,
        etag: ETag,
        updated_at: UtcInstant,
        generated_at: UtcInstant,
    ) -> Result<Self, MismatchedFeedCacheRowFormat> {
        let path_format = feed_path.parts().map(|(_, format)| format);
        let representation_format = representation.format();
        if path_format != Some(representation_format) {
            return Err(MismatchedFeedCacheRowFormat {
                feed_path,
                path_format,
                representation_format,
            });
        }
        Ok(Self {
            feed_path,
            representation,
            etag,
            updated_at,
            generated_at,
        })
    }

    /// Returns the canonical path that keys this cache entry.
    #[must_use]
    pub fn feed_path(&self) -> &FeedPath {
        &self.feed_path
    }

    /// Returns the closed rendered representation and its derived content type.
    #[must_use]
    pub fn representation(&self) -> &SyndicationFeedRepresentation {
        &self.representation
    }

    /// Consumes the row while preserving its exact rendered representation.
    #[must_use]
    pub fn into_representation(self) -> SyndicationFeedRepresentation {
        self.representation
    }
}
#[derive(Debug, Error)]
pub enum FeedCacheError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("stored feed cache metadata conflicts for {feed_path}: {source}")]
    MismatchedStoredMetadata {
        feed_path: FeedPath,
        #[source]
        source: MismatchedStoredSyndicationFeedMetadata,
    },
    #[error("stored feed cache path {0} has no recoverable format")]
    UnrecoverableStoredPath(FeedPath),
}

#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait FeedCacheStorage: Send + Sync {
    async fn get(&self, feed_path: &FeedPath) -> Result<Option<FeedCacheRow>, FeedCacheError>;
    async fn upsert(&self, row: FeedCacheRow) -> Result<(), FeedCacheError>;
    async fn delete(&self, feed_path: &FeedPath) -> Result<(), FeedCacheError>;
}

#[derive(Debug, sqlx::FromRow)]
struct FeedCacheRowRecord {
    feed_url: FeedPath,
    body: String,
    etag: ETag,
    content_type: ContentType,
    updated_at: FeedCacheUpdatedAt,
    generated_at: FeedCacheGeneratedAt,
}

struct FeedCacheRowParts {
    feed_path: FeedPath,
    body: String,
    etag: ETag,
    content_type: ContentType,
    updated_at: FeedCacheUpdatedAt,
    generated_at: FeedCacheGeneratedAt,
}

// `FeedPath` and `ContentType` decode through validating sqlx bridges (#438).
// This mapper establishes the remaining semantic agreement before exposing a row.
fn row_from_record(row: FeedCacheRowRecord) -> Result<FeedCacheRow, FeedCacheError> {
    let parts = FeedCacheRowParts {
        feed_path: row.feed_url,
        body: row.body,
        etag: row.etag,
        content_type: row.content_type,
        updated_at: row.updated_at,
        generated_at: row.generated_at,
    };
    let format = parts
        .feed_path
        .parts()
        .map(|(_, format)| format)
        .ok_or_else(|| FeedCacheError::UnrecoverableStoredPath(parts.feed_path.clone()))?;
    let representation =
        SyndicationFeedRepresentation::try_from_stored(format, parts.content_type, parts.body)
            .map_err(|source| FeedCacheError::MismatchedStoredMetadata {
                feed_path: parts.feed_path.clone(),
                source,
            })?;
    let Ok(row) = FeedCacheRow::new(
        parts.feed_path,
        representation,
        parts.etag,
        parts.updated_at.value(),
        parts.generated_at.value(),
    ) else {
        unreachable!("stored representation and path share the decoded format")
    };
    Ok(row)
}

/// Generic [`FeedCacheStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
pub struct FeedCacheStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> FeedCacheStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> FeedCacheStorage for FeedCacheStore<DB>
where
    DB: Backend,
    FeedCacheRowRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `FeedPath` binds and decodes as itself via the ADR-0071 sqlx bridge (the
    // `feed_url` column decodes into `FeedPath`, and the binds encode `&FeedPath`).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.feed_cache.get",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get(&self, feed_path: &FeedPath) -> Result<Option<FeedCacheRow>, FeedCacheError> {
        let row = sqlx::query_as::<_, FeedCacheRowRecord>(
            "SELECT feed_url, body, etag, content_type, updated_at, generated_at \
             FROM feed_cache WHERE feed_url = $1",
        )
        .bind(feed_path)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_from_record).transpose()
    }

    #[tracing::instrument(
        name = "storage.feed_cache.upsert",
        skip(self, row),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn upsert(&self, row: FeedCacheRow) -> Result<(), FeedCacheError> {
        sqlx::query(
            "INSERT INTO feed_cache (feed_url, body, etag, content_type, updated_at, generated_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT(feed_url) DO UPDATE SET \
               body = excluded.body, \
               etag = excluded.etag, \
               content_type = excluded.content_type, \
               updated_at = excluded.updated_at, \
               generated_at = excluded.generated_at",
        )
        .bind(row.feed_path())
        // The closed representation owns the exact opaque rendered bytes and
        // derives the persisted content type from its format.
        .bind(row.representation().body())
        .bind(&row.etag)
        .bind(row.representation().content_type())
        .bind(row.updated_at)
        .bind(row.generated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.feed_cache.delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete(&self, feed_path: &FeedPath) -> Result<(), FeedCacheError> {
        sqlx::query("DELETE FROM feed_cache WHERE feed_url = $1")
            .bind(feed_path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, backends, fp};

    use common::{feed::FeedFormat, test_support::parse_etag};
    use host::feed::SyndicationFeedRepresentation;
    use rstest::*;
    use rstest_reuse::*;
    fn sample(url: &str) -> FeedCacheRow {
        let feed_path = fp(url);
        let (_, format) = feed_path.parts().expect("valid feed path");
        let updated_at = UtcInstant::now();
        FeedCacheRow::new(
            feed_path,
            SyndicationFeedRepresentation::try_from_stored(
                format,
                format.content_type(),
                "<rss/>".into(),
            )
            .expect("matching stored representation metadata"),
            parse_etag("\"sha256-deadbeef\""),
            updated_at,
            UtcInstant::from(updated_at.value() + chrono::Duration::seconds(5)),
        )
        .expect("matching cache row formats")
    }

    #[test]
    fn timestamp_role_wrappers_preserve_distinct_instants() {
        let updated_at = UtcInstant::now();
        let generated_at = UtcInstant::from(updated_at.value() + chrono::Duration::seconds(5));

        assert_eq!(FeedCacheUpdatedAt::from(updated_at).value(), updated_at);
        assert_eq!(
            FeedCacheGeneratedAt::from(generated_at).value(),
            generated_at
        );
    }

    #[test]
    fn construction_rejects_representation_mismatching_feed_path() {
        let representation = SyndicationFeedRepresentation::try_from_stored(
            FeedFormat::Atom,
            FeedFormat::Atom.content_type(),
            "<feed/>".into(),
        )
        .expect("matching stored representation metadata");
        let updated_at = UtcInstant::now();

        let err = FeedCacheRow::new(
            fp("/feed.rss"),
            representation,
            parse_etag("\"sha256-deadbeef\""),
            updated_at,
            UtcInstant::from(updated_at.value() + chrono::Duration::seconds(5)),
        )
        .expect_err("RSS path must reject Atom representation");

        assert!(matches!(err, MismatchedFeedCacheRowFormat { .. }));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upsert_then_get_roundtrips_adjacent_timestamp_roles_at_microsecond_precision(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let feed_path = fp("/feed.rss");
        let row = FeedCacheRow::new(
            feed_path.clone(),
            SyndicationFeedRepresentation::try_from_stored(
                FeedFormat::Rss,
                FeedFormat::Rss.content_type(),
                "<rss/>".into(),
            )
            .expect("matching stored representation metadata"),
            parse_etag("\"sha256-deadbeef\""),
            "2026-08-25T01:02:03.123456Z"
                .parse()
                .expect("valid UTC instant"),
            "2026-08-25T01:02:03.123457Z"
                .parse()
                .expect("valid UTC instant"),
        )
        .expect("matching cache row formats");
        env.state.feed_cache.upsert(row.clone()).await.unwrap();
        let got = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .expect("present");
        assert_eq!(got.updated_at, row.updated_at);
        assert_eq!(got.generated_at, row.generated_at);
        assert_ne!(got.updated_at, got.generated_at);
        assert_eq!(got.feed_path(), "/feed.rss");
        assert_eq!(got.representation().body(), "<rss/>");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn second_upsert_updates_existing_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let row = sample("/feed.rss");
        env.state.feed_cache.upsert(row.clone()).await.unwrap();
        let replacement = FeedCacheRow::new(
            fp("/feed.rss"),
            SyndicationFeedRepresentation::try_from_stored(
                FeedFormat::Rss,
                FeedFormat::Rss.content_type(),
                "<rss>updated</rss>".into(),
            )
            .expect("matching stored representation metadata"),
            row.etag.clone(),
            row.updated_at,
            row.generated_at,
        )
        .expect("matching cache row formats");
        env.state.feed_cache.upsert(replacement).await.unwrap();
        let got = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.representation().body(), "<rss>updated</rss>");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_surfaces_a_column_decode_error_for_a_malformed_content_type(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        env.state
            .feed_cache
            .upsert(sample("/feed.rss"))
            .await
            .unwrap();
        // A non-media-type value bypasses `ContentType` validation — only reachable via
        // DB tampering. The key stays valid so the row is found; the validating bridge
        // `Decode` (#438) then rejects the `content_type` column on read.
        env.base
            .pool()
            .execute(
                "UPDATE feed_cache SET content_type = 'not-a-content-type' \
                 WHERE feed_url = '/feed.rss'",
            )
            .await
            .unwrap();
        let err = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FeedCacheError::Db(sqlx::Error::ColumnDecode { .. })),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_rejects_directly_inserted_path_content_type_mismatch(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_cache
            .upsert(sample("/feed.rss"))
            .await
            .unwrap();
        env.base
            .pool()
            .execute(
                "UPDATE feed_cache SET content_type = 'application/atom+xml; charset=utf-8' \
                 WHERE feed_url = '/feed.rss'",
            )
            .await
            .unwrap();

        let err = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, FeedCacheError::MismatchedStoredMetadata { .. }),
            "expected a stored-metadata mismatch, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_surfaces_a_column_decode_error_for_a_malformed_etag(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_cache
            .upsert(sample("/feed.rss"))
            .await
            .unwrap();
        // An unquoted value bypasses `ETag`'s quoted-format invariant — only reachable via
        // DB tampering. The key stays valid so the row is found; the validating bridge
        // `Decode` (#438/#634) then rejects the `etag` column on read.
        env.base
            .pool()
            .execute(
                "UPDATE feed_cache SET etag = 'not-a-quoted-etag' \
                 WHERE feed_url = '/feed.rss'",
            )
            .await
            .unwrap();
        let err = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FeedCacheError::Db(sqlx::Error::ColumnDecode { .. })),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_missing_returns_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        assert!(
            env.state
                .feed_cache
                .get(&fp("/tags/absent/feed.rss"))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_removes_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_cache
            .upsert(sample("/feed.rss"))
            .await
            .unwrap();
        env.state.feed_cache.delete(&fp("/feed.rss")).await.unwrap();
        assert!(
            env.state
                .feed_cache
                .get(&fp("/feed.rss"))
                .await
                .unwrap()
                .is_none()
        );
    }
}

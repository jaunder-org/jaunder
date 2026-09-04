//! Cached, fully-rendered feed bodies keyed by their canonical (decoded) path
//! form. The cache layer is the single source of truth for what bytes get
//! served by `GET /feed.{rss,atom,json}` and the other feed endpoints.

use async_trait::async_trait;
use chrono::TimeDelta;
use common::{etag::ETag, feed::FeedFormat, media::ContentType, time::UtcInstant};
use host::{
    etag::FeedSemanticFingerprint,
    feed::{FeedPath, MismatchedStoredSyndicationFeedMetadata, SyndicationFeedRepresentation},
};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::sql::QueryStorageExt;
use crate::{WriteTransaction, backend::Backend, role_instant::impl_role_instant};

/// The `feed_cache.representation_modified_at` storage timestamp role, distinct
/// from `generated_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
pub(crate) struct FeedCacheRepresentationModifiedAt(UtcInstant);
impl_role_instant!(FeedCacheRepresentationModifiedAt, UtcInstant);

/// The `feed_cache.generated_at` storage timestamp role, distinct from
/// `representation_modified_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
pub(crate) struct FeedCacheGeneratedAt(UtcInstant);
impl_role_instant!(FeedCacheGeneratedAt, UtcInstant);

/// A rendered feed body stored and served verbatim until representation validation.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct StoredFeedBody(pub(crate) String);

impl StoredFeedBody {
    fn into_inner(self) -> String {
        self.0
    }
}
/// Validated semantic identity encoded for the storage boundary.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct StoredFeedSemanticFingerprint(String);

impl StoredFeedSemanticFingerprint {
    fn from_fingerprint(fingerprint: &FeedSemanticFingerprint) -> Self {
        Self(fingerprint.to_string())
    }

    fn into_inner(self) -> String {
        self.0
    }
}
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
    pub representation_modified_at: UtcInstant,
    pub generated_at: UtcInstant,
    semantic_fingerprint: FeedSemanticFingerprint,
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
        representation_modified_at: UtcInstant,
        generated_at: UtcInstant,
        semantic_fingerprint: FeedSemanticFingerprint,
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
        let representation_modified_at = UtcInstant::from(
            representation_modified_at.value()
                - TimeDelta::nanoseconds(i64::from(
                    representation_modified_at.value().timestamp_subsec_nanos(),
                )),
        );
        Ok(Self {
            feed_path,
            representation,
            etag,
            representation_modified_at,
            generated_at,
            semantic_fingerprint,
        })
    }
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

    /// Returns the validated semantic identity used for atomic cache replacement.
    #[must_use]
    pub fn semantic_fingerprint(&self) -> &FeedSemanticFingerprint {
        &self.semantic_fingerprint
    }
}
#[derive(Debug, Error)]
pub enum FeedCacheError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("stored feed cache semantic fingerprint is invalid")]
    InvalidSemanticFingerprint,
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
    async fn upsert(
        &self,
        transaction: &mut WriteTransaction,
        row: FeedCacheRow,
    ) -> Result<(), FeedCacheError>;
    async fn delete(
        &self,
        transaction: &mut WriteTransaction,
        feed_path: &FeedPath,
    ) -> Result<(), FeedCacheError>;
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct StoredFeedCacheRow {
    feed_url: FeedPath,
    body: StoredFeedBody,
    etag: ETag,
    content_type: ContentType,
    representation_modified_at: FeedCacheRepresentationModifiedAt,
    generated_at: FeedCacheGeneratedAt,
    semantic_fingerprint: StoredFeedSemanticFingerprint,
}

struct FeedCacheRowParts {
    feed_path: FeedPath,
    body: String,
    etag: ETag,
    content_type: ContentType,
    representation_modified_at: FeedCacheRepresentationModifiedAt,
    generated_at: FeedCacheGeneratedAt,
    semantic_fingerprint: FeedSemanticFingerprint,
}

// `FeedPath` and `ContentType` decode through validating sqlx bridges (#438).
// This mapper establishes the remaining semantic agreement before exposing a row.
fn row_from_stored(row: StoredFeedCacheRow) -> Result<FeedCacheRow, FeedCacheError> {
    let semantic_fingerprint = row
        .semantic_fingerprint
        .into_inner()
        .parse()
        .map_err(|_| FeedCacheError::InvalidSemanticFingerprint)?;
    let parts = FeedCacheRowParts {
        feed_path: row.feed_url,
        body: row.body.into_inner(),
        etag: row.etag,
        content_type: row.content_type,
        representation_modified_at: row.representation_modified_at,
        generated_at: row.generated_at,
        semantic_fingerprint,
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
        parts.representation_modified_at.value(),
        parts.generated_at.value(),
        parts.semantic_fingerprint,
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
    StoredFeedCacheRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `FeedPath` binds and decodes as itself via the ADR-0071 sqlx bridge (the
    // `feed_url` column decodes into `FeedPath`, and the binds encode `&FeedPath`).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.feed_cache.get",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get(&self, feed_path: &FeedPath) -> Result<Option<FeedCacheRow>, FeedCacheError> {
        let row = sqlx::query_as::<_, StoredFeedCacheRow>(
            "SELECT feed_url, body, etag, content_type, representation_modified_at, generated_at, \
             semantic_fingerprint FROM feed_cache WHERE feed_url = $1",
        )
        .bind_storage(feed_path)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_from_stored).transpose()
    }

    #[tracing::instrument(
        name = "storage.feed_cache.upsert",
        skip(self, transaction, row),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn upsert(
        &self,
        transaction: &mut WriteTransaction,
        row: FeedCacheRow,
    ) -> Result<(), FeedCacheError> {
        let connection = DB::write_connection(transaction)?;
        upsert_on_connection::<DB>(connection, row)
            .await
            .map(|_| ())
    }

    #[tracing::instrument(
        name = "storage.feed_cache.delete",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete(
        &self,
        transaction: &mut WriteTransaction,
        feed_path: &FeedPath,
    ) -> Result<(), FeedCacheError> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM feed_cache WHERE feed_url = $1")
            .bind_storage(feed_path)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }
}

/// Writes `row` using an already-acquired short write connection.
///
/// Publisher generation fencing owns the transaction that decides whether this
/// operation is permitted; this helper keeps representation decomposition and
/// feed-cache SQL owned by the cache module.
pub(crate) async fn upsert_on_connection<DB>(
    connection: &mut DB::Connection,
    row: FeedCacheRow,
) -> Result<FeedCacheRow, FeedCacheError>
where
    DB: Database,
    StoredFeedCacheRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    String: sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> ETag: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> ContentType: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> FeedPath: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let body = StoredFeedBody(row.representation().body().to_owned());
    let stored = sqlx::query_as::<_, StoredFeedCacheRow>(
        "INSERT INTO feed_cache \
         (feed_url, body, etag, content_type, representation_modified_at, generated_at, semantic_fingerprint) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT(feed_url) DO UPDATE SET \
         body = CASE WHEN feed_cache.semantic_fingerprint = excluded.semantic_fingerprint \
             THEN feed_cache.body ELSE excluded.body END, \
         etag = CASE WHEN feed_cache.semantic_fingerprint = excluded.semantic_fingerprint \
             THEN feed_cache.etag ELSE excluded.etag END, \
         content_type = CASE WHEN feed_cache.semantic_fingerprint = excluded.semantic_fingerprint \
             THEN feed_cache.content_type ELSE excluded.content_type END, \
         representation_modified_at = CASE \
             WHEN feed_cache.semantic_fingerprint = excluded.semantic_fingerprint \
             THEN feed_cache.representation_modified_at ELSE excluded.representation_modified_at END, \
         generated_at = CASE \
             WHEN feed_cache.semantic_fingerprint = excluded.semantic_fingerprint \
                 AND feed_cache.generated_at > excluded.generated_at \
             THEN feed_cache.generated_at ELSE excluded.generated_at END, \
         semantic_fingerprint = excluded.semantic_fingerprint \
         RETURNING feed_url, body, etag, content_type, representation_modified_at, generated_at, \
         semantic_fingerprint",
    )
    .bind_storage(&row.feed_path)
    .bind_storage(body)
    .bind_storage(&row.etag)
    .bind_storage(row.representation().content_type())
    .bind_storage(row.representation_modified_at)
    .bind_storage(row.generated_at)
    .bind_storage(StoredFeedSemanticFingerprint::from_fingerprint(
        row.semantic_fingerprint(),
    ))
    .fetch_one(connection)
    .await?;
    row_from_stored(stored)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::test_support::{Backend, SeedFeedCache, backends, fp};

    use common::{feed::FeedFormat, test_support::parse_etag};
    use host::feed::SyndicationFeedRepresentation;
    use rstest::*;
    use rstest_reuse::*;

    async fn upsert_confirmed(state: &crate::AppState, row: FeedCacheRow) {
        let cache = Arc::clone(&state.feed_cache);
        let outcome = state
            .write_scope
            .run(move |transaction| Box::pin(async move { cache.upsert(transaction, row).await }))
            .await
            .expect("upsert cache");
        assert!(matches!(
            outcome,
            common::mutation::MutationOutcome::Confirmed(())
        ));
    }

    async fn delete_confirmed(state: &crate::AppState, feed_path: &FeedPath) {
        let cache = Arc::clone(&state.feed_cache);
        let feed_path = feed_path.clone();
        let outcome = state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move { cache.delete(transaction, &feed_path).await })
            })
            .await
            .expect("delete cache");
        assert!(matches!(
            outcome,
            common::mutation::MutationOutcome::Confirmed(())
        ));
    }

    #[test]
    fn timestamp_role_wrappers_preserve_distinct_instants() {
        let updated_at = UtcInstant::now();
        let generated_at = UtcInstant::from(updated_at.value() + chrono::Duration::seconds(5));

        assert_eq!(
            FeedCacheRepresentationModifiedAt(updated_at).value(),
            updated_at
        );
        assert_eq!(FeedCacheGeneratedAt(generated_at).value(), generated_at);
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
            "0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .expect("valid fingerprint"),
        )
        .expect_err("RSS path must reject Atom representation");

        assert!(matches!(err, MismatchedFeedCacheRowFormat { .. }));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upsert_then_get_roundtrips_whole_second_representation_modification_time(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let feed_path = fp("/feed.rss");
        let row = SeedFeedCache::new(feed_path.clone())
            .body("<rss/>".to_owned())
            .etag(parse_etag("\"sha256-deadbeef\""))
            .representation_modified_at(
                "2026-08-25T01:02:03.123456Z"
                    .parse()
                    .expect("valid UTC instant"),
            )
            .generated_at(
                "2026-08-25T01:02:03.123457Z"
                    .parse()
                    .expect("valid UTC instant"),
            )
            .build();
        upsert_confirmed(&env.state, row.clone()).await;
        let got = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .expect("present");
        assert_eq!(
            got.representation_modified_at,
            row.representation_modified_at
        );
        assert_eq!(got.generated_at, row.generated_at);
        assert_eq!(
            got.representation_modified_at
                .value()
                .timestamp_subsec_nanos(),
            0
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn matching_fingerprint_upsert_preserves_existing_representation(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let row = SeedFeedCache::new(fp("/feed.rss"))
            .body("<rss>first</rss>".to_owned())
            .build();
        upsert_confirmed(&env.state, row.clone()).await;
        let replacement = SeedFeedCache::new(fp("/feed.rss"))
            .body("<rss>discarded</rss>".to_owned())
            .etag(parse_etag("\"sha256-rejected-candidate\""))
            .representation_modified_at(row.representation_modified_at)
            .generated_at(UtcInstant::from(
                row.generated_at.value() + chrono::Duration::seconds(1),
            ))
            .build();
        upsert_confirmed(&env.state, replacement).await;
        let got = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.representation().body(), "<rss>first</rss>");
        assert_eq!(got.etag, row.etag);
        assert_eq!(
            got.representation_modified_at,
            row.representation_modified_at
        );
        assert!(got.generated_at > row.generated_at);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn different_fingerprint_upsert_replaces_representation(#[case] backend: Backend) {
        let env = backend.setup().await;
        let row = SeedFeedCache::new(fp("/feed.rss"))
            .body("<rss>first</rss>".to_owned())
            .build();
        upsert_confirmed(&env.state, row).await;
        let replacement = SeedFeedCache::new(fp("/feed.rss"))
            .body("<rss>replacement</rss>".to_owned())
            .semantic_fingerprint(
                "1111111111111111111111111111111111111111111111111111111111111111"
                    .parse()
                    .expect("valid fingerprint"),
            )
            .build();
        upsert_confirmed(&env.state, replacement.clone()).await;
        let got = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, replacement);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_surfaces_a_column_decode_error_for_a_malformed_content_type(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        SeedFeedCache::new(fp("/feed.rss")).seed(&env.state).await;
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
        SeedFeedCache::new(fp("/feed.rss")).seed(&env.state).await;
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
        SeedFeedCache::new(fp("/feed.rss")).seed(&env.state).await;
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
    async fn get_rejects_a_malformed_semantic_fingerprint(#[case] backend: Backend) {
        let env = backend.setup().await;
        SeedFeedCache::new(fp("/feed.rss")).seed(&env.state).await;
        env.base
            .pool()
            .execute(
                "UPDATE feed_cache SET semantic_fingerprint = 'not-a-fingerprint' \
                 WHERE feed_url = '/feed.rss'",
            )
            .await
            .unwrap();

        assert!(matches!(
            env.state.feed_cache.get(&fp("/feed.rss")).await,
            Err(FeedCacheError::InvalidSemanticFingerprint)
        ));
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
    async fn failed_cache_operation_rolls_back_its_write_scope(#[case] backend: Backend) {
        let env = backend.setup().await;
        let row = SeedFeedCache::new(fp("/feed.rss")).build();
        let cache = Arc::clone(&env.state.feed_cache);
        let result = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    cache.upsert(transaction, row).await?;
                    Err::<(), _>(FeedCacheError::Db(sqlx::Error::PoolClosed))
                })
            })
            .await;

        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(FeedCacheError::Db(
                sqlx::Error::PoolClosed
            )))
        ));
        assert!(
            env.state
                .feed_cache
                .get(&fp("/feed.rss"))
                .await
                .expect("read after rolled-back write")
                .is_none()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn indeterminate_cache_commit_is_not_reported_as_confirmed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let scope = env
            .state
            .write_scope
            .with_commit_acknowledgement_loss_after_commit_for_test();
        let cache = Arc::clone(&env.state.feed_cache);
        let row = SeedFeedCache::new(fp("/feed.rss")).build();

        let outcome = scope
            .run(move |transaction| Box::pin(async move { cache.upsert(transaction, row).await }))
            .await
            .expect("cache mutation reaches commit");

        assert!(matches!(
            outcome,
            common::mutation::MutationOutcome::CommitIndeterminate(())
        ));
        assert!(
            env.state
                .feed_cache
                .get(&fp("/feed.rss"))
                .await
                .expect("read after indeterminate commit")
                .is_some()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_removes_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        SeedFeedCache::new(fp("/feed.rss")).seed(&env.state).await;
        delete_confirmed(&env.state, &fp("/feed.rss")).await;
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

//! Cached, fully-rendered feed bodies keyed by their canonical (decoded) path
//! form. The cache layer is the single source of truth for what bytes get
//! served by `GET /feed.{rss,atom,json}` and the other feed endpoints.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::{etag::ETag, feed::FeedPath, media::ContentType};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;

macro_rules! impl_role_instant {
    ($name:ident) => {
        impl $name {
            /// The wrapped UTC instant.
            #[must_use]
            fn value(self) -> DateTime<Utc> {
                self.0
            }
        }

        impl From<DateTime<Utc>> for $name {
            fn from(instant: DateTime<Utc>) -> Self {
                Self(instant)
            }
        }
    };
}

/// The `feed_cache.updated_at` storage timestamp role, distinct from
/// `generated_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct FeedCacheUpdatedAt(DateTime<Utc>);
impl_role_instant!(FeedCacheUpdatedAt);

/// The `feed_cache.generated_at` storage timestamp role, distinct from
/// `updated_at` so mappings cannot transpose silently (#751).
#[derive(Clone, Copy, Debug, PartialEq, Eq, macros::SqlxBridge)]
struct FeedCacheGeneratedAt(DateTime<Utc>);
impl_role_instant!(FeedCacheGeneratedAt);
/// A single cached feed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedCacheRow {
    pub feed_path: FeedPath,
    pub body: String,
    /// The stored strong `ETag`. Decodes through the `ETag` sqlx bridge (#438/#634), so a
    /// corrupt/migrated value is rejected as a `ColumnDecode` error on read-back.
    pub etag: ETag,
    pub content_type: ContentType,
    pub updated_at: DateTime<Utc>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum FeedCacheError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
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

// Infallible: the `feed_url` and `content_type` columns decode straight into
// `FeedPath` / `ContentType` via the sqlx bridge (#438), which validates through
// `FromStr` at the query boundary — so a corrupt/migrated value is already rejected
// as a `ColumnDecode` error before this mapper runs. The adjacent timestamp pair
// decodes through distinct role wrappers (#751), so a swap at this seam fails to compile.
fn row_from_record(row: FeedCacheRowRecord) -> FeedCacheRow {
    let parts = FeedCacheRowParts {
        feed_path: row.feed_url,
        body: row.body,
        etag: row.etag,
        content_type: row.content_type,
        updated_at: row.updated_at,
        generated_at: row.generated_at,
    };
    FeedCacheRow {
        feed_path: parts.feed_path,
        body: parts.body,
        etag: parts.etag,
        content_type: parts.content_type,
        updated_at: parts.updated_at.value(),
        generated_at: parts.generated_at.value(),
    }
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
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
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
        Ok(row.map(row_from_record))
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
        .bind(&row.feed_path)
        // sqlx-newtype-bind:allow permanent-primitive — cached rendered Syndication Feed body is an opaque representation.
        .bind(row.body.as_str())
        .bind(&row.etag)
        .bind(&row.content_type)
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
    use common::test_support::{parse_content_type, parse_etag};
    use rstest::*;
    use rstest_reuse::*;

    fn sample(url: &str) -> FeedCacheRow {
        let updated_at = Utc::now();
        FeedCacheRow {
            feed_path: fp(url),
            body: "<rss/>".into(),
            etag: parse_etag("\"sha256-deadbeef\""),
            content_type: parse_content_type("application/rss+xml"),
            updated_at,
            generated_at: updated_at + chrono::Duration::seconds(5),
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn upsert_then_get_returns_row(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .feed_cache
            .upsert(sample("/feed.rss"))
            .await
            .unwrap();
        let got = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .expect("present");
        assert_eq!(
            got.generated_at,
            got.updated_at + chrono::Duration::seconds(5)
        );
        assert_eq!(got.feed_path, "/feed.rss");
        assert_eq!(got.body, "<rss/>");
    }

    #[apply(backends)]
    #[tokio::test]
    async fn second_upsert_updates_existing_body(#[case] backend: Backend) {
        let env = backend.setup().await;
        let mut row = sample("/feed.rss");
        env.state.feed_cache.upsert(row.clone()).await.unwrap();
        row.body = "<rss>updated</rss>".into();
        env.state.feed_cache.upsert(row).await.unwrap();
        let got = env
            .state
            .feed_cache
            .get(&fp("/feed.rss"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.body, "<rss>updated</rss>");
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

//! Audience storage: named groups of an author's subscriptions, used to target
//! `Named`-visibility content.
//!
//! An audience belongs to exactly one author and carries a unique name within
//! that author (`UNIQUE (author_user_id, name)`). Membership pairs an audience
//! with a subscription; the database guarantees both belong to the **same**
//! author via two composite foreign keys on `audience_members` that each point
//! at the shared `author_user_id` column (migration 0020). The store therefore
//! performs **no** application-level same-owner check — it passes
//! `author_user_id` into the membership insert and lets the FKs reject a
//! cross-author pairing (ADR-0019, same-owner invariant).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::audience::AudienceName;
use sqlx::{Database, Pool};

use crate::backend::Backend;

/// A named audience row returned by [`AudienceStorage::list_audiences`].
#[derive(Clone, Debug)]
pub struct AudienceRecord {
    /// Unique internal identifier.
    pub audience_id: i64,
    /// Author-unique display name.
    pub name: String,
    /// When the audience row was created.
    pub created_at: DateTime<Utc>,
}

/// Failure modes for the mutating audience operations.
#[derive(Debug)]
pub enum AudienceError {
    /// An audience with the same `(author_user_id, name)` already exists.
    DuplicateName,
    /// No audience matched the `(author_user_id, audience_id)` scope.
    NotFound,
    /// Any other storage-layer failure.
    Storage(sqlx::Error),
}

impl From<sqlx::Error> for AudienceError {
    fn from(error: sqlx::Error) -> Self {
        AudienceError::Storage(error)
    }
}

impl From<AudienceError> for host::error::InternalError {
    /// Maps an audience failure to the carrier: duplicate names and missing
    /// audiences are client-correctable; everything else is a masked storage
    /// failure. Reproduces the former `web::audiences::map_audience_error`
    /// `(kind, class, public_message)` exactly, so the wire projection is
    /// preserved by construction.
    fn from(error: AudienceError) -> Self {
        use host::error::InternalError;
        match error {
            AudienceError::DuplicateName => {
                InternalError::conflict("an audience with that name already exists")
            }
            AudienceError::NotFound => InternalError::not_found("audience"),
            AudienceError::Storage(e) => InternalError::storage(e),
        }
    }
}

/// Async operations on the `audiences` / `audience_members` tables.
///
/// Every write is scoped by `author_user_id`; `add_member` additionally threads
/// `author_user_id` into the membership row so the composite FKs enforce the
/// same-owner invariant (no app-level check).
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait AudienceStorage: Send + Sync {
    /// Creates a named audience for the author. Maps the
    /// `UNIQUE (author_user_id, name)` violation to [`AudienceError::DuplicateName`].
    async fn create_audience(
        &self,
        author_user_id: i64,
        name: &AudienceName,
    ) -> Result<i64, AudienceError>;

    /// Renames an audience the author owns. [`AudienceError::NotFound`] if the
    /// `(author_user_id, audience_id)` pair does not exist; [`AudienceError::DuplicateName`]
    /// on a name collision.
    async fn rename_audience(
        &self,
        author_user_id: i64,
        audience_id: i64,
        name: &AudienceName,
    ) -> Result<(), AudienceError>;

    /// Deletes an audience the author owns and its membership rows in one
    /// transaction (the migrations declare no `ON DELETE CASCADE`).
    async fn delete_audience(&self, author_user_id: i64, audience_id: i64) -> sqlx::Result<()>;

    /// Lists the author's audiences, ordered by `audience_id`.
    async fn list_audiences(&self, author_user_id: i64) -> sqlx::Result<Vec<AudienceRecord>>;

    /// Adds a subscription to an audience. `author_user_id` is written into the
    /// row so the composite FKs reject a cross-author pairing at the database
    /// (no app-level same-owner check) — such a rejection surfaces as
    /// [`AudienceError::Storage`].
    async fn add_member(
        &self,
        author_user_id: i64,
        audience_id: i64,
        subscription_id: i64,
    ) -> Result<(), AudienceError>;

    /// Removes a subscription from an audience the author owns. A no-op if absent
    /// (including when `audience_id` belongs to another author).
    async fn remove_member(
        &self,
        author_user_id: i64,
        audience_id: i64,
        subscription_id: i64,
    ) -> sqlx::Result<()>;

    /// Lists the `subscription_id`s belonging to an audience the author owns,
    /// ordered. Empty when `audience_id` belongs to another author.
    async fn list_members(&self, author_user_id: i64, audience_id: i64) -> sqlx::Result<Vec<i64>>;
}

/// Generic [`AudienceStorage`] backed by any [`Backend`] database. The SQL is
/// backend-agnostic — the shared `$n` placeholders bind positionally on both
/// `SQLite` and Postgres — so there is no per-backend dialect: the statements are
/// merged (dialects are split only where a statement genuinely cannot be shared).
pub struct AudienceStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> AudienceStore<DB> {
    /// Constructs a store over the given pool.
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> AudienceStorage for AudienceStore<DB>
where
    DB: Backend,
    // Restated from `Backend` (supertrait where-clauses don't propagate; ADR-0019).
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (i64, String, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.audiences.create",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn create_audience(
        &self,
        author_user_id: i64,
        name: &AudienceName,
    ) -> Result<i64, AudienceError> {
        match sqlx::query_as::<_, (i64,)>(
            "INSERT INTO audiences (author_user_id, name) VALUES ($1, $2) RETURNING audience_id",
        )
        .bind(author_user_id)
        .bind(name.as_ref())
        .fetch_one(&self.pool)
        .await
        {
            Ok((id,)) => Ok(id),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(AudienceError::DuplicateName)
            }
            Err(error) => Err(AudienceError::Storage(error)),
        }
    }

    #[tracing::instrument(
        name = "storage.audiences.rename",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn rename_audience(
        &self,
        author_user_id: i64,
        audience_id: i64,
        name: &AudienceName,
    ) -> Result<(), AudienceError> {
        // `RETURNING` so a no-match is detected generically (via `fetch_optional`)
        // without `rows_affected()`, which sqlx exposes only on concrete results.
        let result = sqlx::query_as::<_, (i64,)>(
            "UPDATE audiences SET name = $1 WHERE author_user_id = $2 AND audience_id = $3 \
             RETURNING audience_id",
        )
        .bind(name.as_ref())
        .bind(author_user_id)
        .bind(audience_id)
        .fetch_optional(&self.pool)
        .await;
        match result {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(AudienceError::NotFound),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(AudienceError::DuplicateName)
            }
            Err(error) => Err(AudienceError::Storage(error)),
        }
    }

    #[tracing::instrument(
        name = "storage.audiences.delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete_audience(&self, author_user_id: i64, audience_id: i64) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM audience_members WHERE author_user_id = $1 AND audience_id = $2")
            .bind(author_user_id)
            .bind(audience_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM audiences WHERE author_user_id = $1 AND audience_id = $2")
            .bind(author_user_id)
            .bind(audience_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.audiences.list",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_audiences(&self, author_user_id: i64) -> sqlx::Result<Vec<AudienceRecord>> {
        let rows = sqlx::query_as::<_, (i64, String, DateTime<Utc>)>(
            "SELECT audience_id, name, created_at FROM audiences \
             WHERE author_user_id = $1 ORDER BY audience_id",
        )
        .bind(author_user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(audience_id, name, created_at)| AudienceRecord {
                audience_id,
                name,
                created_at,
            })
            .collect())
    }

    #[tracing::instrument(
        name = "storage.audiences.add_member",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn add_member(
        &self,
        author_user_id: i64,
        audience_id: i64,
        subscription_id: i64,
    ) -> Result<(), AudienceError> {
        sqlx::query(
            "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (audience_id, subscription_id) DO NOTHING",
        )
        .bind(audience_id)
        .bind(subscription_id)
        .bind(author_user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.audiences.remove_member",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn remove_member(
        &self,
        author_user_id: i64,
        audience_id: i64,
        subscription_id: i64,
    ) -> sqlx::Result<()> {
        sqlx::query(
            "DELETE FROM audience_members \
             WHERE author_user_id = $1 AND audience_id = $2 AND subscription_id = $3",
        )
        .bind(author_user_id)
        .bind(audience_id)
        .bind(subscription_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.audiences.list_members",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_members(&self, author_user_id: i64, audience_id: i64) -> sqlx::Result<Vec<i64>> {
        let rows = sqlx::query_as::<_, (i64,)>(
            "SELECT subscription_id FROM audience_members \
             WHERE author_user_id = $1 AND audience_id = $2 ORDER BY subscription_id",
        )
        .bind(author_user_id)
        .bind(audience_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::AudienceError;
    use host::error::{ErrorKind, InternalError};

    // Behavior-preserving translation of the former `web` `map_audience_error`
    // tests: each variant's `(kind, public_message)` is what the deleted mapper
    // produced, so the wire projection is unchanged.
    #[test]
    fn from_audience_error_maps_variants() {
        let duplicate: InternalError = AudienceError::DuplicateName.into();
        assert_eq!(duplicate.kind(), ErrorKind::Conflict);
        assert_eq!(
            duplicate.public_message(),
            "an audience with that name already exists"
        );

        let not_found: InternalError = AudienceError::NotFound.into();
        assert_eq!(not_found.kind(), ErrorKind::NotFound);
        assert_eq!(not_found.public_message(), "audience not found");

        let storage: InternalError = AudienceError::Storage(sqlx::Error::PoolClosed).into();
        assert_eq!(storage.kind(), ErrorKind::Storage);
        assert_eq!(storage.public_message(), "storage operation failed");
    }
}

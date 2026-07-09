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
use sqlx::{Database, Pool};

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
    async fn create_audience(&self, author_user_id: i64, name: &str) -> Result<i64, AudienceError>;

    /// Renames an audience the author owns. [`AudienceError::NotFound`] if the
    /// `(author_user_id, audience_id)` pair does not exist; [`AudienceError::DuplicateName`]
    /// on a name collision.
    async fn rename_audience(
        &self,
        author_user_id: i64,
        audience_id: i64,
        name: &str,
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

    /// Removes a subscription from an audience. A no-op if absent.
    async fn remove_member(&self, audience_id: i64, subscription_id: i64) -> sqlx::Result<()>;

    /// Lists the `subscription_id`s belonging to an audience, ordered.
    async fn list_members(&self, audience_id: i64) -> sqlx::Result<Vec<i64>>;
}

/// Per-backend SQL for [`AudienceStore`]. The statements differ only in the
/// placeholder syntax (`SQLite` `?`, Postgres `$n`); the logical behavior is
/// identical (ADR-0019).
pub trait AudienceDialect: Database {
    /// Inserts an audience and returns its id. Bind order: `author_user_id, name`.
    const INSERT_AUDIENCE: &'static str;
    /// Renames an audience scoped to its owner, returning the affected
    /// `audience_id` (`RETURNING` so a no-match is detected generically without
    /// `rows_affected()`, which sqlx exposes only on concrete result types).
    /// Bind order: `name, author_user_id, audience_id`.
    const RENAME_AUDIENCE: &'static str;
    /// Deletes the audience's membership rows. Bind order: `author_user_id, audience_id`.
    const DELETE_AUDIENCE_MEMBERS: &'static str;
    /// Deletes the audience scoped to its owner. Bind order: `author_user_id, audience_id`.
    const DELETE_AUDIENCE: &'static str;
    /// Lists the author's audiences. Bind order: `author_user_id`.
    const LIST_AUDIENCES: &'static str;
    /// Idempotent membership insert carrying the owner id for the composite FKs.
    /// Bind order: `audience_id, subscription_id, author_user_id`.
    const INSERT_MEMBER: &'static str;
    /// Removes a membership row. Bind order: `audience_id, subscription_id`.
    const DELETE_MEMBER: &'static str;
    /// Lists an audience's `subscription_id`s. Bind order: `audience_id`.
    const LIST_MEMBERS: &'static str;
}

/// Generic [`AudienceStorage`] backed by any database implementing
/// [`AudienceDialect`]. Backend SQL is supplied by the dialect; see ADR-0019.
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
    DB: AudienceDialect,
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (i64, String, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_audience(&self, author_user_id: i64, name: &str) -> Result<i64, AudienceError> {
        match sqlx::query_as::<_, (i64,)>(DB::INSERT_AUDIENCE)
            .bind(author_user_id)
            .bind(name)
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

    async fn rename_audience(
        &self,
        author_user_id: i64,
        audience_id: i64,
        name: &str,
    ) -> Result<(), AudienceError> {
        let result = sqlx::query_as::<_, (i64,)>(DB::RENAME_AUDIENCE)
            .bind(name)
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

    async fn delete_audience(&self, author_user_id: i64, audience_id: i64) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(DB::DELETE_AUDIENCE_MEMBERS)
            .bind(author_user_id)
            .bind(audience_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(DB::DELETE_AUDIENCE)
            .bind(author_user_id)
            .bind(audience_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn list_audiences(&self, author_user_id: i64) -> sqlx::Result<Vec<AudienceRecord>> {
        let rows = sqlx::query_as::<_, (i64, String, DateTime<Utc>)>(DB::LIST_AUDIENCES)
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

    async fn add_member(
        &self,
        author_user_id: i64,
        audience_id: i64,
        subscription_id: i64,
    ) -> Result<(), AudienceError> {
        sqlx::query(DB::INSERT_MEMBER)
            .bind(audience_id)
            .bind(subscription_id)
            .bind(author_user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn remove_member(&self, audience_id: i64, subscription_id: i64) -> sqlx::Result<()> {
        sqlx::query(DB::DELETE_MEMBER)
            .bind(audience_id)
            .bind(subscription_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_members(&self, audience_id: i64) -> sqlx::Result<Vec<i64>> {
        let rows = sqlx::query_as::<_, (i64,)>(DB::LIST_MEMBERS)
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

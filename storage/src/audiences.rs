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

use crate::WriteTransaction;
use crate::sql::QueryStorageExt;
use async_trait::async_trait;
use common::audience::AudienceName;
use common::ids::{AudienceId, SubscriptionId, UserId};
use common::time::UtcInstant;
use sqlx::{Database, Pool, Row};
use std::collections::BTreeSet;

use crate::backend::Backend;

/// A named audience row returned by [`AudienceStorage::list_audiences`].
#[derive(Clone, Debug)]
pub struct AudienceRecord {
    /// Unique internal identifier.
    pub audience_id: AudienceId,
    /// Author-unique display name.
    pub name: AudienceName,
    /// When the audience row was created.
    pub created_at: UtcInstant,
}

impl<'r, R> sqlx::FromRow<'r, R> for AudienceRecord
where
    R: Row,
    &'r str: sqlx::ColumnIndex<R>,
    AudienceId: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    AudienceName: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    UtcInstant: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    fn from_row(row: &'r R) -> sqlx::Result<Self> {
        let audience_id = row.try_get::<AudienceId, _>("audience_id")?;
        let name = row.try_get::<AudienceName, _>("name")?;
        let created_at = row.try_get::<UtcInstant, _>("created_at")?;

        Ok(Self {
            audience_id,
            name,
            created_at,
        })
    }
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
/// Failure from validating named audience targets for a particular author.
///
/// [`Invalid`](Self::Invalid) deliberately covers both foreign and nonexistent
/// audience identifiers. Callers must project it as one opaque validation error.
#[derive(Debug)]
pub enum InvalidAudienceTargets {
    /// At least one named target is not owned by the author.
    Invalid,
    /// Listing the author's audiences failed.
    Storage(sqlx::Error),
}

impl From<InvalidAudienceTargets> for host::error::InternalError {
    fn from(error: InvalidAudienceTargets) -> Self {
        match error {
            InvalidAudienceTargets::Invalid => {
                host::error::InternalError::validation("invalid audience")
            }
            InvalidAudienceTargets::Storage(error) => host::error::InternalError::storage(error),
        }
    }
}

/// Validates that each named audience target belongs to `author_user_id`.
///
/// The lookup is author-scoped, so a foreign identifier and an identifier that
/// does not exist produce the same [`InvalidAudienceTargets::Invalid`] error.
///
/// # Errors
///
/// Returns [`InvalidAudienceTargets::Invalid`] when any named target is not
/// owned by the author, or [`InvalidAudienceTargets::Storage`] when the
/// author-scoped lookup fails.
pub async fn validate_named_audience_targets(
    storage: &dyn AudienceStorage,
    author_user_id: UserId,
    targets: &[common::visibility::AudienceTarget],
) -> Result<(), InvalidAudienceTargets> {
    let named: BTreeSet<_> = targets
        .iter()
        .filter_map(|target| match target {
            common::visibility::AudienceTarget::Named(id) => Some(*id),
            common::visibility::AudienceTarget::Public
            | common::visibility::AudienceTarget::Private
            | common::visibility::AudienceTarget::Subscribers => None,
        })
        .collect();
    if named.is_empty() {
        return Ok(());
    }

    let allowed: BTreeSet<_> = storage
        .list_audiences(author_user_id)
        .await
        .map_err(InvalidAudienceTargets::Storage)?
        .into_iter()
        .map(|audience| audience.audience_id)
        .collect();
    if named.is_subset(&allowed) {
        Ok(())
    } else {
        Err(InvalidAudienceTargets::Invalid)
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
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        name: &AudienceName,
    ) -> Result<AudienceId, AudienceError>;

    /// Renames an audience the author owns. [`AudienceError::NotFound`] if the
    /// `(author_user_id, audience_id)` pair does not exist; [`AudienceError::DuplicateName`]
    /// on a name collision.
    async fn rename_audience(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
        name: &AudienceName,
    ) -> Result<(), AudienceError>;
    /// Deletes an audience the author owns and its membership rows in one
    /// transaction (the migrations declare no `ON DELETE CASCADE`).
    async fn delete_audience(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
    ) -> sqlx::Result<()>;
    /// Lists the author's audiences, ordered by `audience_id`.
    async fn list_audiences(&self, author_user_id: UserId) -> sqlx::Result<Vec<AudienceRecord>>;

    /// Adds a subscription to an audience. `author_user_id` is written into the
    /// row so the composite FKs reject a cross-author pairing at the database
    /// (no app-level same-owner check) — such a rejection surfaces as
    /// [`AudienceError::Storage`].
    async fn add_member(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
        subscription_id: SubscriptionId,
    ) -> Result<(), AudienceError>;

    /// Removes a subscription from an audience the author owns. A no-op if absent
    /// (including when `audience_id` belongs to another author).
    async fn remove_member(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
        subscription_id: SubscriptionId,
    ) -> sqlx::Result<()>;

    /// Lists the `subscription_id`s belonging to an audience the author owns,
    /// ordered. Empty when `audience_id` belongs to another author.
    async fn list_members(
        &self,
        author_user_id: UserId,
        audience_id: AudienceId,
    ) -> sqlx::Result<Vec<SubscriptionId>>;
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
    (AudienceId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SubscriptionId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    AudienceRecord: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `AudienceName` binds and decodes as itself via the ADR-0071 sqlx bridge
    // (the `name` column decodes into `AudienceName`, and the create/rename binds
    // encode `&AudienceName`).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
    for<'r> UtcInstant: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
{
    #[tracing::instrument(
        name = "storage.audiences.create",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn create_audience(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        name: &AudienceName,
    ) -> Result<AudienceId, AudienceError> {
        let connection = DB::write_connection(transaction).map_err(AudienceError::Storage)?;
        match sqlx::query_as::<_, (AudienceId,)>(
            "INSERT INTO audiences (author_user_id, name) VALUES ($1, $2) RETURNING audience_id",
        )
        .bind_storage(author_user_id)
        .bind_storage(name)
        .fetch_one(&mut *connection)
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
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn rename_audience(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
        name: &AudienceName,
    ) -> Result<(), AudienceError> {
        // `RETURNING` so a no-match is detected generically (via `fetch_optional`)
        // without `rows_affected()`, which sqlx exposes only on concrete results.
        let connection = DB::write_connection(transaction).map_err(AudienceError::Storage)?;
        let result = sqlx::query_as::<_, (AudienceId,)>(
            "UPDATE audiences SET name = $1 WHERE author_user_id = $2 AND audience_id = $3 \
             RETURNING audience_id",
        )
        .bind_storage(name)
        .bind_storage(author_user_id)
        .bind_storage(audience_id)
        .fetch_optional(&mut *connection)
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
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete_audience(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM audience_members WHERE author_user_id = $1 AND audience_id = $2")
            .bind_storage(author_user_id)
            .bind_storage(audience_id)
            .execute(&mut *connection)
            .await?;
        sqlx::query("DELETE FROM audiences WHERE author_user_id = $1 AND audience_id = $2")
            .bind_storage(author_user_id)
            .bind_storage(audience_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.audiences.list",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_audiences(&self, author_user_id: UserId) -> sqlx::Result<Vec<AudienceRecord>> {
        sqlx::query_as::<_, AudienceRecord>(
            "SELECT audience_id, name, created_at FROM audiences \
             WHERE author_user_id = $1 ORDER BY audience_id",
        )
        .bind_storage(author_user_id)
        .fetch_all(&self.pool)
        .await
    }

    #[tracing::instrument(
        name = "storage.audiences.add_member",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn add_member(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
        subscription_id: SubscriptionId,
    ) -> Result<(), AudienceError> {
        let connection = DB::write_connection(transaction).map_err(AudienceError::Storage)?;
        sqlx::query(
            "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (audience_id, subscription_id) DO NOTHING",
        )
        .bind_storage(audience_id)
        .bind_storage(subscription_id)
        .bind_storage(author_user_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.audiences.remove_member",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn remove_member(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        audience_id: AudienceId,
        subscription_id: SubscriptionId,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query(
            "DELETE FROM audience_members \
             WHERE author_user_id = $1 AND audience_id = $2 AND subscription_id = $3",
        )
        .bind_storage(author_user_id)
        .bind_storage(audience_id)
        .bind_storage(subscription_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    #[tracing::instrument(
        name = "storage.audiences.list_members",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn list_members(
        &self,
        author_user_id: UserId,
        audience_id: AudienceId,
    ) -> sqlx::Result<Vec<SubscriptionId>> {
        let rows = sqlx::query_as::<_, (SubscriptionId,)>(
            "SELECT subscription_id FROM audience_members \
             WHERE author_user_id = $1 AND audience_id = $2 ORDER BY subscription_id",
        )
        .bind_storage(author_user_id)
        .bind_storage(audience_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{AudienceError, InvalidAudienceTargets, validate_named_audience_targets};
    use crate::sql::QueryStorageExt;
    use crate::test_support::{Backend, CloseablePool, SeedUser, backends};
    use common::audience::AudienceName;
    use common::ids::AudienceId;
    use common::test_support::parse_audience_name;
    use common::time::UtcInstant;
    use common::visibility::AudienceTarget;
    use host::error::{ErrorKind, InternalError};
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    async fn create_audience_confirmed(
        state: &Arc<crate::AppState>,
        author_user_id: common::ids::UserId,
        name: &AudienceName,
    ) -> AudienceId {
        let audiences = Arc::clone(&state.audiences);
        let write_scope = state.write_scope.clone();
        let name = name.clone();
        crate::test_support::confirmed_for(
            write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        audiences
                            .create_audience(transaction, author_user_id, &name)
                            .await
                    })
                })
                .await
                .unwrap(),
            "audience fixture setup",
        )
    }

    enum BlockedAudienceWrite {
        Create,
        Rename,
    }

    async fn block_audience_write(
        backend: Backend,
        pool: &CloseablePool,
        write: BlockedAudienceWrite,
    ) {
        match (backend, write) {
            (Backend::Sqlite, BlockedAudienceWrite::Create) => pool
                .execute(
                    "CREATE TRIGGER block_audience_write \
                     BEFORE INSERT ON audiences \
                     BEGIN SELECT RAISE(FAIL, 'blocked'); END",
                )
                .await
                .unwrap(),
            (Backend::Sqlite, BlockedAudienceWrite::Rename) => pool
                .execute(
                    "CREATE TRIGGER block_audience_write \
                     BEFORE UPDATE ON audiences \
                     BEGIN SELECT RAISE(FAIL, 'blocked'); END",
                )
                .await
                .unwrap(),
            (Backend::Postgres, write) => {
                pool.execute(
                    "CREATE FUNCTION block_audience_write() RETURNS trigger AS $$ \
                     BEGIN RAISE EXCEPTION 'blocked'; END; $$ LANGUAGE plpgsql",
                )
                .await
                .unwrap();
                match write {
                    BlockedAudienceWrite::Create => pool
                        .execute(
                            "CREATE TRIGGER block_audience_write \
                             BEFORE INSERT ON audiences \
                             FOR EACH ROW EXECUTE FUNCTION block_audience_write()",
                        )
                        .await
                        .unwrap(),
                    BlockedAudienceWrite::Rename => pool
                        .execute(
                            "CREATE TRIGGER block_audience_write \
                             BEFORE UPDATE ON audiences \
                             FOR EACH ROW EXECUTE FUNCTION block_audience_write()",
                        )
                        .await
                        .unwrap(),
                }
            }
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_audience_preserves_non_unique_database_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        let author_user_id = SeedUser::new().seed(&env.state).await.user_id;
        block_audience_write(backend, env.base.pool(), BlockedAudienceWrite::Create).await;
        let audiences = Arc::clone(&env.state.audiences);
        let result = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    audiences
                        .create_audience(
                            transaction,
                            author_user_id,
                            &parse_audience_name("Blocked"),
                        )
                        .await
                })
            })
            .await;

        assert!(
            matches!(
                &result,
                Err(crate::WriteScopeError::Operation(AudienceError::Storage(
                    sqlx::Error::Database(error)
                ))) if !error.is_unique_violation()
            ),
            "expected a non-unique database error to remain a storage error, got {result:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn rename_audience_preserves_non_unique_database_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        let author_user_id = SeedUser::new().seed(&env.state).await.user_id;
        let audience_id =
            create_audience_confirmed(&env.state, author_user_id, &parse_audience_name("Original"))
                .await;
        block_audience_write(backend, env.base.pool(), BlockedAudienceWrite::Rename).await;
        let audiences = Arc::clone(&env.state.audiences);
        let result = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    audiences
                        .rename_audience(
                            transaction,
                            author_user_id,
                            audience_id,
                            &parse_audience_name("Blocked"),
                        )
                        .await
                })
            })
            .await;

        assert!(
            matches!(
                &result,
                Err(crate::WriteScopeError::Operation(AudienceError::Storage(
                    sqlx::Error::Database(error)
                ))) if !error.is_unique_violation()
            ),
            "expected a non-unique database error to remain a storage error, got {result:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn audience_created_at_round_trips_and_list_preserves_id_order(#[case] backend: Backend) {
        let env = backend.setup().await;
        let author = SeedUser::new().seed(&env.state).await.user_id;
        let first_id =
            create_audience_confirmed(&env.state, author, &parse_audience_name("Close Friends"))
                .await;
        let second_id =
            create_audience_confirmed(&env.state, author, &parse_audience_name("Family")).await;
        let first_created_at = "2026-01-02T03:04:05.654321Z".parse::<UtcInstant>().unwrap();
        let second_created_at = "2026-01-02T03:04:05.123456Z".parse::<UtcInstant>().unwrap();

        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("UPDATE audiences SET created_at = $1 WHERE audience_id = $2")
                .bind_storage(first_created_at)
                .bind_storage(first_id)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("UPDATE audiences SET created_at = $1 WHERE audience_id = $2")
                .bind_storage(second_created_at)
                .bind_storage(second_id)
                .execute(pool)
                .await
                .unwrap();
        });

        let listed = env.state.audiences.list_audiences(author).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].audience_id, first_id);
        assert_eq!(listed[0].created_at, first_created_at);
        assert_eq!(listed[1].audience_id, second_id);
        assert_eq!(listed[1].created_at, second_created_at);
        assert!(listed[0].created_at > listed[1].created_at);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn list_audiences_surfaces_a_column_decode_error_for_a_malformed_name(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let author = SeedUser::new().seed(&env.state).await.user_id;
        // A whitespace-only name bypasses `AudienceName` validation (which
        // `create_audience` enforces) — only reachable via DB tampering. The
        // validating bridge `Decode` rejects it on read as a column-decode error.
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query("INSERT INTO audiences (author_user_id, name) VALUES ($1, '   ')")
                .bind_storage(author)
                .execute(pool)
                .await
                .map(|_| ())
        })
        .unwrap();
        let err = env
            .state
            .audiences
            .list_audiences(author)
            .await
            .unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn named_target_validation_is_author_scoped_and_opaque(#[case] backend: Backend) {
        let env = backend.setup().await;
        let author = SeedUser::new().seed(&env.state).await;
        let other = SeedUser::new().seed(&env.state).await;
        let owned =
            create_audience_confirmed(&env.state, author.user_id, &parse_audience_name("Owned"))
                .await;
        let foreign =
            create_audience_confirmed(&env.state, other.user_id, &parse_audience_name("Foreign"))
                .await;

        validate_named_audience_targets(
            env.state.audiences.as_ref(),
            author.user_id,
            &[AudienceTarget::Public, AudienceTarget::Named(owned)],
        )
        .await
        .unwrap();

        let foreign = validate_named_audience_targets(
            env.state.audiences.as_ref(),
            author.user_id,
            &[AudienceTarget::Named(foreign)],
        )
        .await
        .unwrap_err();
        let unknown = validate_named_audience_targets(
            env.state.audiences.as_ref(),
            author.user_id,
            &[AudienceTarget::Named(common::ids::AudienceId::from(
                999_999,
            ))],
        )
        .await
        .unwrap_err();
        assert!(matches!(foreign, InvalidAudienceTargets::Invalid));
        assert!(matches!(unknown, InvalidAudienceTargets::Invalid));
    }

    // Each variant's `(kind, public_message)` is the wire projection; these pin it.
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

    #[test]
    fn invalid_audience_target_storage_failure_is_masked_as_storage() {
        let error: InternalError = InvalidAudienceTargets::Storage(sqlx::Error::PoolClosed).into();
        assert_eq!(error.kind(), ErrorKind::Storage);
        assert_eq!(error.public_message(), "storage operation failed");
    }
}

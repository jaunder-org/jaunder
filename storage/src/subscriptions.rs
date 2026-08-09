//! Subscription storage: who follows whom on which channel, and the
//! admission seam that decides a new subscription's initial status.
//!
//! The store routes every `subscribe` through a [`SubscriptionPolicy`] (the
//! admission seam — see ADR-0020). Layer A wires the [`OpenSubscriptionPolicy`]
//! (auto-approve to `active`); later milestones swap in an approval gate without
//! touching this store. `is_subscriber` admits only `active` rows, so a row left
//! `pending`/`blocked` by a stricter policy fails closed.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};

use common::ids::{ChannelId, SubscriptionId, UserId};
use common::visibility::{
    SubscriptionPolicy, SubscriptionStatus, ViewerIdentity, local_subscriber_ref,
};

use crate::error::{StorageError, fetch_exactly_one_scalar};

/// A subscription row returned by [`SubscriptionStorage::list_subscribers`].
#[derive(Clone, Debug)]
pub struct SubscriptionRecord {
    /// Unique internal identifier.
    pub subscription_id: SubscriptionId,
    /// Channel the subscription is on (e.g. the `local` channel).
    pub channel_id: ChannelId,
    /// Channel-scoped opaque reference to the subscriber (the local user id,
    /// rendered as a string, for the `local` channel).
    pub subscriber_ref: String,
    /// Current admission status.
    pub status: SubscriptionStatus,
    /// When the subscription row was created.
    pub created_at: DateTime<Utc>,
}

/// Async operations on the `subscriptions` table.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait SubscriptionStorage: Send + Sync {
    /// Routes through the admission seam to pick the initial status, then
    /// upserts idempotently. Returns the (possibly pre-existing) `subscription_id`.
    async fn subscribe(
        &self,
        author_user_id: UserId,
        channel_id: ChannelId,
        subscriber_ref: &str,
    ) -> Result<SubscriptionId, StorageError>;

    /// Removes a subscription. A no-op if it does not exist.
    async fn unsubscribe(
        &self,
        author_user_id: UserId,
        channel_id: ChannelId,
        subscriber_ref: &str,
    ) -> Result<(), StorageError>;

    /// Returns `true` only for an `active` subscription matching the viewer.
    /// `Anonymous` short-circuits to `Ok(false)` without a query.
    async fn is_subscriber(
        &self,
        author_user_id: UserId,
        viewer: &ViewerIdentity,
    ) -> Result<bool, StorageError>;

    /// Lists the author's `active` subscribers.
    async fn list_subscribers(
        &self,
        author_user_id: UserId,
    ) -> Result<Vec<SubscriptionRecord>, StorageError>;

    /// Returns the `channel_id` of the seeded `local` channel.
    ///
    /// This serves the subscription **write** path only — `subscribe` and
    /// `unsubscribe` key their rows by channel, so a caller inserting or
    /// deleting one has to name it. Read paths must not use it: the resolution
    /// filter and `is_subscriber` resolve the local channel inline in SQL
    /// (#6), which is both cheaper than a round trip and impossible to point at
    /// the wrong row.
    ///
    /// Returns [`StorageError::MissingRow`] if the seed is absent — a broken
    /// install, named rather than lifted as an anonymous driver error (#343).
    async fn local_channel_id(&self) -> Result<ChannelId, StorageError>;
}

/// Per-backend SQL for [`SubscriptionStore`]. The statements differ only in the
/// placeholder syntax (`SQLite` `?`, Postgres `$n`); the logical behavior is
/// identical (ADR-0019).
pub trait SubscriptionDialect: Database {
    /// Idempotent upsert: resolves the status name to its `status_id` via a
    /// subquery and **returns the `subscription_id` on both paths** — the fresh
    /// insert and the `(author_user_id, channel_id, subscriber_ref)` conflict.
    ///
    /// The conflict arm is a deliberate no-op write
    /// (`SET subscriber_ref = excluded.subscriber_ref`) rather than `DO NOTHING`,
    /// because `DO NOTHING` returns no row and so cannot feed `RETURNING`. That
    /// is what previously forced a second `SELECT`, and with it a window where a
    /// concurrent delete made the id unrecoverable (#343). `status_id` is
    /// deliberately absent from the `SET` list, so an existing subscription keeps
    /// its status — the outcome `DO NOTHING` gave.
    ///
    /// Bind order: `author_user_id, channel_id, subscriber_ref, status_name`.
    const INSERT_SUBSCRIPTION: &'static str;
    /// Deletes the row for the unique triple. Bind order:
    /// `author_user_id, channel_id, subscriber_ref`.
    const DELETE_SUBSCRIPTION: &'static str;
    /// `EXISTS` of an `active` subscription for the triple. Bind order:
    /// `author_user_id, channel_id, subscriber_ref`.
    const IS_ACTIVE_SUBSCRIBER: &'static str;
    /// `EXISTS` of an `active` subscription on the seeded `local` channel, whose
    /// id is resolved by subquery rather than bound — a local viewer's channel is
    /// never a free parameter (ADR-0020, #6). Bind order:
    /// `author_user_id, subscriber_ref`.
    const IS_ACTIVE_LOCAL_SUBSCRIBER: &'static str;
    /// Lists the author's `active` subscriptions. Bind order: `author_user_id`.
    const LIST_ACTIVE_SUBSCRIBERS: &'static str;
    /// Selects the `channel_id` of the seeded `local` channel. No binds.
    const SELECT_LOCAL_CHANNEL_ID: &'static str;
}

/// Generic [`SubscriptionStorage`] backed by any database implementing
/// [`SubscriptionDialect`].
///
/// Holds the pool **and** the admission [`SubscriptionPolicy`]; backend SQL is
/// supplied by [`SubscriptionDialect`]. See ADR-0019 / ADR-0020.
pub struct SubscriptionStore<DB: Database> {
    pool: Pool<DB>,
    policy: Arc<dyn SubscriptionPolicy>,
}

impl<DB: Database> SubscriptionStore<DB> {
    /// Constructs a store with an explicit admission policy.
    #[must_use]
    pub fn new(pool: Pool<DB>, policy: Arc<dyn SubscriptionPolicy>) -> Self {
        Self { pool, policy }
    }
}

#[async_trait]
impl<DB> SubscriptionStorage for SubscriptionStore<DB>
where
    DB: SubscriptionDialect,
    // `IS_ACTIVE_SUBSCRIBER` yields an existence flag, not an id — it stays `i64`.
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SubscriptionId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (ChannelId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SubscriptionId, ChannelId, String, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn subscribe(
        &self,
        author_user_id: UserId,
        channel_id: ChannelId,
        subscriber_ref: &str,
    ) -> Result<SubscriptionId, StorageError> {
        let status = self
            .policy
            .initial_status(author_user_id, channel_id, subscriber_ref);
        // The insert resolves the status *name* to its FK `status_id` (the column is
        // an integer FK, not a TEXT-token enum column). Bind the name as a typed
        // `&'static str` (strum `IntoStaticStr`) — not a stringly `.as_str()` strip.
        let status_name: &'static str = status.into();
        // One statement, not two. The upsert's `RETURNING` yields the id on both
        // the insert and the conflict path, so there is no window in which a
        // concurrent delete can strand us without an id (#343).
        // `RETURNING` on both arms makes the row guaranteed, so the `MissingRow`
        // arm is unreachable today — it is still routed through the wrapper, so
        // the day someone turns the conflict arm back into `DO NOTHING` the
        // absence is reported by name rather than as `RowNotFound` (#343).
        fetch_exactly_one_scalar(
            sqlx::query_scalar::<_, SubscriptionId>(DB::INSERT_SUBSCRIPTION)
                .bind(author_user_id)
                .bind(channel_id)
                .bind(subscriber_ref)
                .bind(status_name),
            &self.pool,
            "the upserted subscription row",
        )
        .await
    }

    async fn unsubscribe(
        &self,
        author_user_id: UserId,
        channel_id: ChannelId,
        subscriber_ref: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(DB::DELETE_SUBSCRIPTION)
            .bind(author_user_id)
            .bind(channel_id)
            .bind(subscriber_ref)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn is_subscriber(
        &self,
        author_user_id: UserId,
        viewer: &ViewerIdentity,
    ) -> Result<bool, StorageError> {
        // Bind arity is per-variant: a local viewer's channel is the seeded
        // `local` row, resolved inside `IS_ACTIVE_LOCAL_SUBSCRIBER` rather than
        // bound, so that arm has one fewer bind (#6).
        //
        // Both statements are `EXISTS`-shaped and so always yield a row; they go
        // through the wrapper anyway, because a bare `fetch_one` is the thing
        // being removed and an unreachable named arm costs nothing (#343).
        let exists = match viewer {
            ViewerIdentity::Anonymous => return Ok(false), // short-circuit; no query.
            // A local viewer carries no channel: it can only ever be the
            // `local` row, which `IS_ACTIVE_LOCAL_SUBSCRIBER` resolves itself.
            ViewerIdentity::Local { user_id } => {
                let subscriber_ref = local_subscriber_ref(*user_id);
                fetch_exactly_one_scalar(
                    sqlx::query_scalar::<_, i64>(DB::IS_ACTIVE_LOCAL_SUBSCRIBER)
                        .bind(author_user_id)
                        .bind(subscriber_ref.as_str()),
                    &self.pool,
                    "the local viewer's active-subscription existence flag",
                )
                .await?
            }
            ViewerIdentity::Remote {
                channel_id,
                subscriber_ref,
            } => {
                fetch_exactly_one_scalar(
                    sqlx::query_scalar::<_, i64>(DB::IS_ACTIVE_SUBSCRIBER)
                        .bind(author_user_id)
                        .bind(*channel_id)
                        .bind(subscriber_ref.as_str()),
                    &self.pool,
                    "the remote viewer's active-subscription existence flag",
                )
                .await?
            }
        };
        Ok(exists != 0)
    }

    async fn list_subscribers(
        &self,
        author_user_id: UserId,
    ) -> Result<Vec<SubscriptionRecord>, StorageError> {
        let rows = sqlx::query_as::<_, (SubscriptionId, ChannelId, String, DateTime<Utc>)>(
            DB::LIST_ACTIVE_SUBSCRIBERS,
        )
        .bind(author_user_id)
        .fetch_all(&self.pool)
        .await?;
        // The query filters to `st.name = 'active'`, so every returned row is an
        // active subscription — no per-row status decoding needed.
        Ok(rows
            .into_iter()
            .map(
                |(subscription_id, channel_id, subscriber_ref, created_at)| SubscriptionRecord {
                    subscription_id,
                    channel_id,
                    subscriber_ref,
                    status: SubscriptionStatus::Active,
                    created_at,
                },
            )
            .collect())
    }

    async fn local_channel_id(&self) -> Result<ChannelId, StorageError> {
        fetch_exactly_one_scalar(
            sqlx::query_scalar::<_, ChannelId>(DB::SELECT_LOCAL_CHANNEL_ID),
            &self.pool,
            "the seeded 'local' channel row",
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, TestEnv, backends};
    use rstest::*;
    use rstest_reuse::*;

    /// Covers [`StorageError::Db`] — the wrapper's *other* arm.
    ///
    /// The `MissingRow` arm is proven end-to-end by
    /// `local_channel_id_names_the_row_when_the_seed_is_missing` in
    /// `server/tests/storage`. This pins that a genuine driver failure still
    /// classifies as `Db`, so the wrapper cannot quietly report every failure as
    /// a missing row (#343, spec AC13).
    #[apply(backends)]
    #[tokio::test]
    async fn local_channel_id_with_closed_pool_reports_a_driver_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let result = state.subscriptions.local_channel_id().await;
        assert!(
            matches!(result, Err(StorageError::Db(_))),
            "a closed pool is a driver failure, not an absent row"
        );
    }

    /// Guards the two dialect constants against drifting apart — the failure mode
    /// where one backend gains the local-channel statement and the other is
    /// forgotten, which passes `SQLite` and fails Postgres (ADR-0019, #6).
    ///
    /// A *sync* check, not a semantic one: the behaviour it guards is proven on
    /// both backends by the `is_subscriber` tests in `server/tests/storage`.
    #[test]
    fn is_active_local_subscriber_resolves_the_channel_on_both_dialects() {
        for (name, sql) in [
            (
                "sqlite",
                <sqlx::Sqlite as SubscriptionDialect>::IS_ACTIVE_LOCAL_SUBSCRIBER,
            ),
            (
                "postgres",
                <sqlx::Postgres as SubscriptionDialect>::IS_ACTIVE_LOCAL_SUBSCRIBER,
            ),
        ] {
            assert!(
                sql.contains("(SELECT channel_id FROM channels WHERE name = 'local')"),
                "{name} must resolve the local channel inline: {sql}"
            );
        }
    }
}

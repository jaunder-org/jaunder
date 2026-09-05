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
use common::ids::{ChannelId, SubscriptionId, UserId};
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::{
    self, InvalidSubscriberRef, SubscriberIdentity, SubscriberRef, SubscriptionPolicy,
    SubscriptionStatus, ViewerIdentity,
};
use host::error::InternalResult;
use sqlx::{Database, Decode, Encode, Executor, Pool, Result, Row, Type};

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::error::RequireRow;
use crate::sql::QueryStorageExt;
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct SubscriptionStatusName(String);

use crate::sql::Exists;
/// Test-only invalid `subscriptions.subscriber_ref` column value.
///
/// This role deliberately bypasses `SubscriberRef` validation for fixtures that
/// verify strict database/read boundaries without admitting arbitrary text to
/// production persistence.
#[cfg(any(test, feature = "test-support"))]
#[derive(macros::SqlxBridge)]
pub struct CorruptSubscriberRef(pub String);

/// A subscription row returned by [`SubscriptionStorage::list_subscribers`].
#[derive(Clone, Debug)]
pub struct SubscriptionRecord {
    /// Unique internal identifier.
    pub subscription_id: SubscriptionId,
    /// Persisted channel identity of the subscriber.
    pub subscriber: SubscriberIdentity,
    /// Current admission status.
    pub status: SubscriptionStatus,
    /// When the subscription row was created.
    pub created_at: UtcInstant,
}

/// A subscriber row projected for named-audience presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscriberSummaryRecord {
    pub subscription_id: SubscriptionId,
    pub label: String,
}

fn invalid_subscriber_ref_decode(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::ColumnDecode { source, .. }
            if source.downcast_ref::<InvalidSubscriberRef>().is_some()
    )
}

/// Async operations on the `subscriptions` table.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait SubscriptionStorage: Send + Sync {
    /// Routes through the admission seam to pick the initial status, then
    /// upserts idempotently. Returns the (possibly pre-existing) `subscription_id`.
    async fn subscribe(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        subscriber: &SubscriberIdentity,
    ) -> Result<SubscriptionId>;

    /// Removes a subscription. A no-op if it does not exist.
    async fn unsubscribe(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        subscriber: &SubscriberIdentity,
    ) -> Result<()>;

    /// Returns `true` only for an `active` subscription matching the viewer.
    /// `Anonymous` short-circuits to `Ok(false)` without a query.
    async fn is_subscriber(&self, author_user_id: UserId, viewer: &ViewerIdentity) -> Result<bool>;

    /// Lists the author's `active` subscribers.
    async fn list_subscribers(&self, author_user_id: UserId) -> Result<Vec<SubscriptionRecord>>;

    /// Lists the author's active subscribers with the display label resolved.
    async fn list_subscriber_summaries(
        &self,
        author_user_id: UserId,
    ) -> Result<Vec<SubscriberSummaryRecord>>;

    /// Returns the `channel_id` of the seeded `local` channel.
    ///
    /// This serves the subscription **write** path only — `subscribe` and
    /// `unsubscribe` key their rows by channel, so a caller inserting or
    /// deleting one has to name it. Read paths must not use it: the resolution
    /// filter and `is_subscriber` resolve the local channel inline in SQL
    /// (#6), which is both cheaper than a round trip and impossible to point at
    /// the wrong row.
    ///
    /// Unlike its siblings this returns an [`InternalResult`], because it is the
    /// one method here that reads a row which can genuinely be absent: a
    /// database whose `local` seed is gone. That absence is named — the
    /// operator is told which row — rather than surfacing as an anonymous
    /// driver error (#343). It still pages; a missing seed is a broken install.
    async fn local_channel_id(&self) -> InternalResult<ChannelId>;
}

/// Per-backend marker for [`SubscriptionStore`].
///
/// The statements intentionally use the shared SQL subset both backends accept:
/// numbered `$n` bind markers (accepted by `SQLite` and required by `Postgres`) and
/// standard `CAST(... AS TEXT)` rather than backend-specific `::text`.
pub trait SubscriptionDialect: Database {
    /// Idempotent upsert: resolves the status name to its `status_id` via a
    /// subquery and, on the `(author_user_id, channel_id, subscriber_ref)`
    /// conflict, rewrites `subscriber_ref` to the value it already holds. That
    /// deliberate no-op write is what makes `RETURNING` emit the row on the
    /// conflict path too — so the statement returns the `subscription_id` on
    /// both paths, and no second `SELECT` (and no TOCTOU window) is needed.
    /// `status_id` stays out of the `SET` list, so an existing subscription
    /// keeps its status. Bind order:
    /// `author_user_id, channel_id, subscriber_ref, status_name`.
    const INSERT_SUBSCRIPTION: &'static str = "INSERT INTO subscriptions \
         (author_user_id, channel_id, subscriber_ref, status_id) \
         VALUES ($1, $2, $3, (SELECT status_id FROM subscription_statuses WHERE name = $4)) \
         ON CONFLICT (author_user_id, channel_id, subscriber_ref) \
         DO UPDATE SET subscriber_ref = excluded.subscriber_ref \
         RETURNING subscription_id";
    /// Deletes the row for the unique triple. Bind order:
    /// `author_user_id, channel_id, subscriber_ref`.
    const DELETE_SUBSCRIPTION: &'static str = "DELETE FROM subscriptions \
         WHERE author_user_id = $1 AND channel_id = $2 AND subscriber_ref = $3";
    /// `EXISTS` of an `active` subscription for the triple. Bind order:
    /// `author_user_id, channel_id, subscriber_ref`.
    const IS_ACTIVE_SUBSCRIBER: &'static str = "SELECT EXISTS( \
           SELECT 1 FROM subscriptions s \
           JOIN subscription_statuses st ON st.status_id = s.status_id \
           WHERE s.author_user_id = $1 AND s.channel_id = $2 AND s.subscriber_ref = $3 \
             AND st.name = 'active')";
    /// `EXISTS` of an `active` subscription on the seeded `local` channel, whose
    /// id is resolved by subquery rather than bound — a local viewer's channel is
    /// never a free parameter (ADR-0020, #6). Bind order:
    /// `author_user_id, subscriber_ref`.
    const IS_ACTIVE_LOCAL_SUBSCRIBER: &'static str = "SELECT EXISTS( \
           SELECT 1 FROM subscriptions s \
           JOIN subscription_statuses st ON st.status_id = s.status_id \
           WHERE s.author_user_id = $1 \
             AND s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local') \
             AND s.subscriber_ref = $2 \
             AND st.name = 'active')";
    /// Lists the author's `active` subscriptions. Bind order: `author_user_id`.
    const LIST_ACTIVE_SUBSCRIBERS: &'static str = "SELECT \
           s.subscription_id, s.channel_id, s.subscriber_ref, s.created_at \
         FROM subscriptions s \
         JOIN subscription_statuses st ON st.status_id = s.status_id \
         WHERE s.author_user_id = $1 AND st.name = 'active' \
         ORDER BY s.subscription_id";
    /// Lists active subscribers with local-user display labels resolved after
    /// both the optional username and raw subscriber reference have crossed
    /// their typed decode boundaries. Bind order: `author_user_id`.
    const LIST_SUBSCRIBER_SUMMARIES: &'static str = "SELECT \
           s.subscription_id, u.username, s.subscriber_ref \
        FROM subscriptions s \
        JOIN subscription_statuses st ON st.status_id = s.status_id \
        LEFT JOIN users u \
          ON s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local') \
         AND s.subscriber_ref = CAST(u.user_id AS TEXT) \
        WHERE s.author_user_id = $1 AND st.name = 'active' \
        ORDER BY s.subscription_id";
    /// Selects the `channel_id` of the seeded `local` channel. No binds.
    const SELECT_LOCAL_CHANNEL_ID: &'static str =
        "SELECT channel_id FROM channels WHERE name = 'local'";
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
    DB: SubscriptionDialect + Backend,
    (Exists,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (SubscriptionId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (ChannelId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'r> SubscriptionId: Decode<'r, DB> + Type<DB>,
    for<'r> ChannelId: Decode<'r, DB> + Type<DB>,
    for<'r> SubscriberRef: Decode<'r, DB> + Type<DB>,
    for<'r> Username: Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: Decode<'r, DB> + Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q SubscriberRef: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    async fn subscribe(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        subscriber: &SubscriberIdentity,
    ) -> Result<SubscriptionId> {
        let connection = DB::write_connection(transaction)?;
        let status = self.policy.initial_status(author_user_id, subscriber);
        // The insert resolves the status *name* to its FK `status_id` (the column is
        // an integer FK, not a TEXT-token enum column). Bind the name as a typed
        // `&'static str` (strum `IntoStaticStr`) — not a stringly `.as_str()` strip.
        let status_name: &'static str = status.into();
        // One statement, not an insert followed by a select: the separate
        // `SELECT` could miss a row a concurrent unsubscribe had just deleted
        // (#343). `RETURNING` fires on the insert arm and on the `DO UPDATE`
        // conflict arm alike, so the row is guaranteed and `fetch_one` is the
        // honest read.
        sqlx::query_as::<_, (SubscriptionId,)>(DB::INSERT_SUBSCRIPTION)
            .bind_storage(author_user_id)
            .bind_storage(subscriber.channel_id)
            .bind_storage(&subscriber.subscriber_ref)
            .bind_storage(SubscriptionStatusName(status_name.into()))
            .fetch_one(&mut *connection)
            .await
            .map(|(id,)| id)
    }

    async fn unsubscribe(
        &self,
        transaction: &mut WriteTransaction,
        author_user_id: UserId,
        subscriber: &SubscriberIdentity,
    ) -> Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query(DB::DELETE_SUBSCRIPTION)
            .bind_storage(author_user_id)
            .bind_storage(subscriber.channel_id)
            .bind_storage(&subscriber.subscriber_ref)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn is_subscriber(&self, author_user_id: UserId, viewer: &ViewerIdentity) -> Result<bool> {
        // Bind arity is per-variant: a local viewer's channel is the seeded
        // `local` row, resolved inside `IS_ACTIVE_LOCAL_SUBSCRIBER` rather than
        // bound, so that arm has one fewer bind (#6).
        let (exists,) = match viewer {
            ViewerIdentity::Anonymous => return Ok(false), // short-circuit; no query.
            // A local viewer carries no channel: it can only ever be the
            // `local` row, which `IS_ACTIVE_LOCAL_SUBSCRIBER` resolves itself.
            ViewerIdentity::Local { user_id } => {
                let subscriber_ref = visibility::local_subscriber_ref(*user_id);
                sqlx::query_as::<_, (Exists,)>(DB::IS_ACTIVE_LOCAL_SUBSCRIBER)
                    .bind_storage(author_user_id)
                    .bind_storage(&subscriber_ref)
                    .fetch_one(&self.pool)
                    .await?
            }
            ViewerIdentity::Remote {
                channel_id,
                subscriber_ref,
            } => {
                sqlx::query_as::<_, (Exists,)>(DB::IS_ACTIVE_SUBSCRIBER)
                    .bind_storage(author_user_id)
                    .bind_storage(*channel_id)
                    .bind_storage(subscriber_ref)
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(exists.into_bool())
    }

    async fn list_subscribers(&self, author_user_id: UserId) -> Result<Vec<SubscriptionRecord>> {
        // Decode every unrelated column before the one this bulk-read policy
        // may skip. That keeps the diversion column-scoped: an invalid
        // `subscriber_ref` costs only its row, while identity, channel, and
        // timestamp failures still fail the batch (ADR-0122).
        let rows = sqlx::query(DB::LIST_ACTIVE_SUBSCRIBERS)
            .bind_storage(author_user_id)
            .fetch_all(&self.pool)
            .await?;
        let mut records = Vec::with_capacity(rows.len());
        let mut decode_reported = false;
        for row in rows {
            let subscription_id: SubscriptionId = row.try_get("subscription_id")?;
            let channel_id: ChannelId = row.try_get("channel_id")?;
            let created_at: UtcInstant = row.try_get("created_at")?;
            let subscriber_ref = match row.try_get::<SubscriberRef, _>("subscriber_ref") {
                Ok(subscriber_ref) => subscriber_ref,
                Err(error) if invalid_subscriber_ref_decode(&error) => {
                    if !decode_reported {
                        host::error::report_swallowed(
                            host::error::ErrorKind::Storage,
                            host::error::ErrorClass::Bug,
                            "storage.subscriptions.decode_subscriber_ref",
                            host::error::SwallowedSource::Error(&error),
                        );
                        decode_reported = true;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            records.push(SubscriptionRecord {
                subscription_id,
                subscriber: SubscriberIdentity::new(channel_id, subscriber_ref),
                // The query filters to `st.name = 'active'`.
                status: SubscriptionStatus::Active,
                created_at,
            });
        }
        Ok(records)
    }

    async fn list_subscriber_summaries(
        &self,
        author_user_id: UserId,
    ) -> Result<Vec<SubscriberSummaryRecord>> {
        // As above, decode every non-divertible column first so only the
        // validated subscriber reference can make this summary skip a row.
        let rows = sqlx::query(DB::LIST_SUBSCRIBER_SUMMARIES)
            .bind_storage(author_user_id)
            .fetch_all(&self.pool)
            .await?;
        let mut summaries = Vec::with_capacity(rows.len());
        let mut decode_reported = false;
        for row in rows {
            let subscription_id: SubscriptionId = row.try_get("subscription_id")?;
            let username: Option<Username> = row.try_get("username")?;
            let subscriber_ref = match row.try_get::<SubscriberRef, _>("subscriber_ref") {
                Ok(subscriber_ref) => subscriber_ref,
                Err(error) if invalid_subscriber_ref_decode(&error) => {
                    if !decode_reported {
                        host::error::report_swallowed(
                            host::error::ErrorKind::Storage,
                            host::error::ErrorClass::Bug,
                            "storage.subscriptions.decode_summary_subscriber_ref",
                            host::error::SwallowedSource::Error(&error),
                        );
                        decode_reported = true;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            summaries.push(SubscriberSummaryRecord {
                subscription_id,
                label: username.map_or_else(|| String::from(subscriber_ref), String::from),
            });
        }
        Ok(summaries)
    }

    async fn local_channel_id(&self) -> InternalResult<ChannelId> {
        let row = sqlx::query_as::<_, (ChannelId,)>(DB::SELECT_LOCAL_CHANNEL_ID)
            .fetch_optional(&self.pool)
            .await?;
        let (id,) = row.require_row("the seeded 'local' channel row")?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

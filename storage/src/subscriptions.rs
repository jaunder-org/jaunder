//! Subscription storage: who follows whom on which channel, and the
//! admission seam that decides a new subscription's initial status.
//!
//! The store routes every `subscribe` through a [`SubscriptionPolicy`] (the
//! admission seam — see ADR-0020). Layer A wires the [`OpenSubscriptionPolicy`]
//! (auto-approve to `active`); later milestones swap in an approval gate without
//! touching this store. `is_subscriber` admits only `active` rows, so a row left
//! `pending`/`blocked` by a stricter policy fails closed.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};

use common::ids::UserId;
use common::visibility::{SubscriptionPolicy, SubscriptionStatus, ViewerIdentity};

/// A subscription row returned by [`SubscriptionStorage::list_subscribers`].
#[derive(Clone, Debug)]
pub struct SubscriptionRecord {
    /// Unique internal identifier.
    pub subscription_id: i64,
    /// Channel the subscription is on (e.g. the `local` channel).
    pub channel_id: i64,
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
        channel_id: i64,
        subscriber_ref: &str,
    ) -> sqlx::Result<i64>;

    /// Removes a subscription. A no-op if it does not exist.
    async fn unsubscribe(
        &self,
        author_user_id: UserId,
        channel_id: i64,
        subscriber_ref: &str,
    ) -> sqlx::Result<()>;

    /// Returns `true` only for an `active` subscription matching the viewer.
    /// `Anonymous` short-circuits to `Ok(false)` without a query.
    async fn is_subscriber(
        &self,
        author_user_id: UserId,
        viewer: &ViewerIdentity,
    ) -> sqlx::Result<bool>;

    /// Lists the author's `active` subscribers.
    async fn list_subscribers(
        &self,
        author_user_id: UserId,
    ) -> sqlx::Result<Vec<SubscriptionRecord>>;

    /// Returns the `channel_id` of the seeded `local` channel.
    ///
    /// This is the production lookup the web `viewer_identity()` extractor and
    /// `subscribe_to` use to build a [`ViewerIdentity::local`]. The read path
    /// memoizes the result once per process (see [`local_channel_id`]) rather
    /// than querying per request.
    async fn local_channel_id(&self) -> sqlx::Result<i64>;
}

/// Process-level cache of the seeded `local` channel id.
///
/// The `local` channel is created once by migration `0018` and never changes,
/// so a single lookup is reused for the life of the process instead of querying
/// `channels` on every read request.
static LOCAL_CHANNEL_ID: OnceLock<i64> = OnceLock::new();

/// Looks up the seeded `local` channel id, memoizing it for the process.
///
/// The lookup runs at most once per process on the happy path: once the
/// [`OnceLock`](std::sync::OnceLock) is populated it is returned without touching
/// storage. A storage error leaves the cell empty (the next request retries) and
/// yields `None` — **fail-closed**: the web `viewer_identity` adapter treats a
/// viewer whose channel it cannot resolve as anonymous, so it sees only public
/// content.
pub async fn local_channel_id(subscriptions: &dyn SubscriptionStorage) -> Option<i64> {
    if let Some(id) = LOCAL_CHANNEL_ID.get() {
        return Some(*id);
    }
    let id = subscriptions.local_channel_id().await.ok()?;
    // Race-loser's value is identical (the row is immutable), so ignore the Err.
    let _ = LOCAL_CHANNEL_ID.set(id);
    Some(id)
}

/// Per-backend SQL for [`SubscriptionStore`]. The statements differ only in the
/// placeholder syntax (`SQLite` `?`, Postgres `$n`); the logical behavior is
/// identical (ADR-0019).
pub trait SubscriptionDialect: Database {
    /// Idempotent insert: resolves the status name to its `status_id` via a
    /// subquery and no-ops on the `(author_user_id, channel_id, subscriber_ref)`
    /// conflict. Bind order: `author_user_id, channel_id, subscriber_ref, status_name`.
    const INSERT_SUBSCRIPTION: &'static str;
    /// Selects the `subscription_id` for the unique triple. Bind order:
    /// `author_user_id, channel_id, subscriber_ref`.
    const SELECT_SUBSCRIPTION_ID: &'static str;
    /// Deletes the row for the unique triple. Bind order:
    /// `author_user_id, channel_id, subscriber_ref`.
    const DELETE_SUBSCRIPTION: &'static str;
    /// `EXISTS` of an `active` subscription for the triple. Bind order:
    /// `author_user_id, channel_id, subscriber_ref`.
    const IS_ACTIVE_SUBSCRIBER: &'static str;
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
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    (i64, i64, String, DateTime<Utc>): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn subscribe(
        &self,
        author_user_id: UserId,
        channel_id: i64,
        subscriber_ref: &str,
    ) -> sqlx::Result<i64> {
        let status = self
            .policy
            .initial_status(author_user_id, channel_id, subscriber_ref);
        sqlx::query(DB::INSERT_SUBSCRIPTION)
            .bind(i64::from(author_user_id))
            .bind(channel_id)
            .bind(subscriber_ref)
            .bind(status.as_str())
            .execute(&self.pool)
            .await?;
        sqlx::query_as::<_, (i64,)>(DB::SELECT_SUBSCRIPTION_ID)
            .bind(i64::from(author_user_id))
            .bind(channel_id)
            .bind(subscriber_ref)
            .fetch_one(&self.pool)
            .await
            .map(|(id,)| id)
    }

    async fn unsubscribe(
        &self,
        author_user_id: UserId,
        channel_id: i64,
        subscriber_ref: &str,
    ) -> sqlx::Result<()> {
        sqlx::query(DB::DELETE_SUBSCRIPTION)
            .bind(i64::from(author_user_id))
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
    ) -> sqlx::Result<bool> {
        let ViewerIdentity::Channel {
            channel_id,
            subscriber_ref,
        } = viewer
        else {
            return Ok(false); // Anonymous short-circuit; no query.
        };
        let (exists,) = sqlx::query_as::<_, (i64,)>(DB::IS_ACTIVE_SUBSCRIBER)
            .bind(i64::from(author_user_id))
            .bind(channel_id)
            .bind(subscriber_ref.as_str())
            .fetch_one(&self.pool)
            .await?;
        Ok(exists != 0)
    }

    async fn list_subscribers(
        &self,
        author_user_id: UserId,
    ) -> sqlx::Result<Vec<SubscriptionRecord>> {
        let rows =
            sqlx::query_as::<_, (i64, i64, String, DateTime<Utc>)>(DB::LIST_ACTIVE_SUBSCRIBERS)
                .bind(i64::from(author_user_id))
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

    async fn local_channel_id(&self) -> sqlx::Result<i64> {
        sqlx::query_as::<_, (i64,)>(DB::SELECT_LOCAL_CHANNEL_ID)
            .fetch_one(&self.pool)
            .await
            .map(|(id,)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend};
    use rstest::*;
    use rstest_reuse::*;

    // Functional check only — memoization is deliberately not asserted here:
    // `LOCAL_CHANNEL_ID` is a process-global `OnceLock`, so under a
    // two-backends-one-process run the first backend's value would leak into the
    // second. We compare the fail-closed helper against the same backend's direct
    // trait lookup, which stays correct regardless of memoization.
    #[apply(backends)]
    #[tokio::test]
    async fn local_channel_id_returns_the_seeded_local_channel(#[case] backend: Backend) {
        let env = backend.setup().await;
        let expected = env
            .state
            .subscriptions
            .local_channel_id()
            .await
            .expect("migration seeds the local channel");
        assert_eq!(
            local_channel_id(env.state.subscriptions.as_ref()).await,
            Some(expected),
            "the fail-closed helper resolves the seeded local channel id",
        );
    }
}

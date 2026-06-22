use sqlx::Postgres;

use crate::subscriptions::{SubscriptionDialect, SubscriptionStore};

/// Postgres-backed subscription storage.
pub type PostgresSubscriptionStorage = SubscriptionStore<Postgres>;

impl SubscriptionDialect for Postgres {
    const INSERT_SUBSCRIPTION: &'static str = "INSERT INTO subscriptions \
         (author_user_id, channel_id, subscriber_ref, status_id) \
         VALUES ($1, $2, $3, (SELECT status_id FROM subscription_statuses WHERE name = $4)) \
         ON CONFLICT (author_user_id, channel_id, subscriber_ref) DO NOTHING";

    const SELECT_SUBSCRIPTION_ID: &'static str = "SELECT subscription_id FROM subscriptions \
         WHERE author_user_id = $1 AND channel_id = $2 AND subscriber_ref = $3";

    const DELETE_SUBSCRIPTION: &'static str = "DELETE FROM subscriptions \
         WHERE author_user_id = $1 AND channel_id = $2 AND subscriber_ref = $3";

    // Postgres `EXISTS` yields boolean and forbids a direct boolean→bigint cast,
    // so map to 0/1 via CASE — matching SQLite's integer `EXISTS` result so both
    // backends decode into the same `(i64,)` row (ADR-0019).
    const IS_ACTIVE_SUBSCRIBER: &'static str = "SELECT CASE WHEN EXISTS( \
           SELECT 1 FROM subscriptions s \
           JOIN subscription_statuses st ON st.status_id = s.status_id \
           WHERE s.author_user_id = $1 AND s.channel_id = $2 AND s.subscriber_ref = $3 \
             AND st.name = 'active') THEN 1::bigint ELSE 0::bigint END";

    const LIST_ACTIVE_SUBSCRIBERS: &'static str = "SELECT \
           s.subscription_id, s.channel_id, s.subscriber_ref, s.created_at \
         FROM subscriptions s \
         JOIN subscription_statuses st ON st.status_id = s.status_id \
         WHERE s.author_user_id = $1 AND st.name = 'active' \
         ORDER BY s.subscription_id";

    const SELECT_LOCAL_CHANNEL_ID: &'static str =
        "SELECT channel_id FROM channels WHERE name = 'local'";
}

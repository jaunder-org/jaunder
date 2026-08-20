use sqlx::Sqlite;

use crate::subscriptions::{SubscriptionDialect, SubscriptionStore};

/// SQLite-backed subscription storage.
pub type SqliteSubscriptionStorage = SubscriptionStore<Sqlite>;

impl SubscriptionDialect for Sqlite {
    const INSERT_SUBSCRIPTION: &'static str = "INSERT INTO subscriptions \
         (author_user_id, channel_id, subscriber_ref, status_id) \
         VALUES (?, ?, ?, (SELECT status_id FROM subscription_statuses WHERE name = ?)) \
         ON CONFLICT (author_user_id, channel_id, subscriber_ref) \
         DO UPDATE SET subscriber_ref = excluded.subscriber_ref \
         RETURNING subscription_id";

    const DELETE_SUBSCRIPTION: &'static str = "DELETE FROM subscriptions \
         WHERE author_user_id = ? AND channel_id = ? AND subscriber_ref = ?";

    const IS_ACTIVE_SUBSCRIBER: &'static str = "SELECT EXISTS( \
           SELECT 1 FROM subscriptions s \
           JOIN subscription_statuses st ON st.status_id = s.status_id \
           WHERE s.author_user_id = ? AND s.channel_id = ? AND s.subscriber_ref = ? \
             AND st.name = 'active')";

    const IS_ACTIVE_LOCAL_SUBSCRIBER: &'static str = "SELECT EXISTS( \
           SELECT 1 FROM subscriptions s \
           JOIN subscription_statuses st ON st.status_id = s.status_id \
           WHERE s.author_user_id = ? \
             AND s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local') \
             AND s.subscriber_ref = ? \
             AND st.name = 'active')";

    const LIST_ACTIVE_SUBSCRIBERS: &'static str = "SELECT \
           s.subscription_id, s.channel_id, s.subscriber_ref, s.created_at \
         FROM subscriptions s \
         JOIN subscription_statuses st ON st.status_id = s.status_id \
         WHERE s.author_user_id = ? AND st.name = 'active' \
         ORDER BY s.subscription_id";

    const LIST_SUBSCRIBER_SUMMARIES: &'static str = "SELECT \
           s.subscription_id, COALESCE(u.username, s.subscriber_ref) AS label \
        FROM subscriptions s \
        JOIN subscription_statuses st ON st.status_id = s.status_id \
        LEFT JOIN users u \
          ON s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local') \
         AND s.subscriber_ref = CAST(u.user_id AS TEXT) \
        WHERE s.author_user_id = ? AND st.name = 'active' \
        ORDER BY s.subscription_id";

    const SELECT_LOCAL_CHANNEL_ID: &'static str =
        "SELECT channel_id FROM channels WHERE name = 'local'";
}

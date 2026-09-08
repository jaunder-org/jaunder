use sqlx::Sqlite;

use crate::subscriptions::SubscriptionStore;

/// SQLite-backed subscription storage.
pub type SqliteSubscriptionStorage = SubscriptionStore<Sqlite>;

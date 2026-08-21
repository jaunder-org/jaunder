use sqlx::Sqlite;

use crate::subscriptions::{SubscriptionDialect, SubscriptionStore};

/// SQLite-backed subscription storage.
pub type SqliteSubscriptionStorage = SubscriptionStore<Sqlite>;

impl SubscriptionDialect for Sqlite {}

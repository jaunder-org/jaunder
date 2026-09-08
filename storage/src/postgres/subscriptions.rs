use sqlx::Postgres;

use crate::subscriptions::SubscriptionStore;

/// Postgres-backed subscription storage.
pub type PostgresSubscriptionStorage = SubscriptionStore<Postgres>;

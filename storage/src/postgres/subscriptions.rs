use sqlx::Postgres;

use crate::subscriptions::{SubscriptionDialect, SubscriptionStore};

/// Postgres-backed subscription storage.
pub type PostgresSubscriptionStorage = SubscriptionStore<Postgres>;

impl SubscriptionDialect for Postgres {}

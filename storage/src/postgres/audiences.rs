use sqlx::Postgres;

use crate::audiences::AudienceStore;

/// Postgres-backed audience storage.
pub type PostgresAudienceStorage = AudienceStore<Postgres>;

use sqlx::Postgres;

use crate::user_config::UserConfigStore;

/// Postgres-backed user-config storage.
pub type PostgresUserConfigStorage = UserConfigStore<Postgres>;

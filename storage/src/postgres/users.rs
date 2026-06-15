use sqlx::Postgres;

use crate::users::UserStore;

/// Postgres-backed user storage.
pub type PostgresUserStorage = UserStore<Postgres>;

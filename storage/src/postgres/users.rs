use sqlx::Postgres;

use crate::users::{UserDialect, UserStore};

/// Postgres-backed user storage.
pub type PostgresUserStorage = UserStore<Postgres>;

impl UserDialect for Postgres {
    const FOR_UPDATE: &'static str = " FOR UPDATE";
}

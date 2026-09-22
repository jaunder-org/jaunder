use sqlx::Postgres;

use crate::user_config::{UserConfigDialect, UserConfigStore};

/// Postgres-backed user-config storage.
pub type PostgresUserConfigStorage = UserConfigStore<Postgres>;

impl UserConfigDialect for Postgres {
    const FOR_UPDATE: &'static str = " FOR UPDATE";
}

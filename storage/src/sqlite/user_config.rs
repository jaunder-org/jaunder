use sqlx::Sqlite;

use crate::user_config::{UserConfigDialect, UserConfigStore};

/// SQLite-backed user-config storage.
pub type SqliteUserConfigStorage = UserConfigStore<Sqlite>;

impl UserConfigDialect for Sqlite {
    const FOR_UPDATE: &'static str = "";
}

use sqlx::Sqlite;

use crate::user_config::UserConfigStore;

/// SQLite-backed user-config storage.
pub type SqliteUserConfigStorage = UserConfigStore<Sqlite>;

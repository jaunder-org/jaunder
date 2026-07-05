use sqlx::Sqlite;

use crate::users::UserStore;

/// SQLite-backed user storage.
pub type SqliteUserStorage = UserStore<Sqlite>;

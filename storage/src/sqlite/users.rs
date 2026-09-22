use sqlx::Sqlite;

use crate::users::{UserDialect, UserStore};

/// SQLite-backed user storage.
pub type SqliteUserStorage = UserStore<Sqlite>;

impl UserDialect for Sqlite {
    const FOR_UPDATE: &'static str = "";
}

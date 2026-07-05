use sqlx::Sqlite;

use crate::password::PasswordResetStore;

/// SQLite-backed password-reset token storage.
pub type SqlitePasswordResetStorage = PasswordResetStore<Sqlite>;

use sqlx::Sqlite;

use crate::email::EmailVerificationStore;

/// SQLite-backed email verification token storage.
pub type SqliteEmailVerificationStorage = EmailVerificationStore<Sqlite>;

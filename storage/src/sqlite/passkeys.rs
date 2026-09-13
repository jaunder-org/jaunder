use sqlx::Sqlite;

use crate::passkeys::PasskeyStore;

/// SQLite-backed Passkey storage.
pub type SqlitePasskeyStorage = PasskeyStore<Sqlite>;

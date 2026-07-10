use sqlx::Sqlite;

use crate::audiences::AudienceStore;

/// SQLite-backed audience storage.
pub type SqliteAudienceStorage = AudienceStore<Sqlite>;

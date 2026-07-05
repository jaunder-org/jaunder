use sqlx::Sqlite;

use crate::invites::InviteStore;

/// SQLite-backed invite code storage.
pub type SqliteInviteStorage = InviteStore<Sqlite>;

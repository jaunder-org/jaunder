use sqlx::Postgres;

use crate::invites::InviteStore;

/// Postgres-backed invite code storage.
pub type PostgresInviteStorage = InviteStore<Postgres>;

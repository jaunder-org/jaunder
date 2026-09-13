use sqlx::Postgres;

use crate::passkeys::PasskeyStore;

/// PostgreSQL-backed Passkey storage.
pub type PostgresPasskeyStorage = PasskeyStore<Postgres>;

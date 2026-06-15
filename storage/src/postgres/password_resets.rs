use sqlx::Postgres;

use crate::password::PasswordResetStore;

/// Postgres-backed password-reset token storage.
pub type PostgresPasswordResetStorage = PasswordResetStore<Postgres>;

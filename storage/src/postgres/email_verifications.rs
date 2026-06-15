use sqlx::Postgres;

use crate::email::EmailVerificationStore;

/// Postgres-backed email verification token storage.
pub type PostgresEmailVerificationStorage = EmailVerificationStore<Postgres>;

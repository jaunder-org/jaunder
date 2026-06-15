//! Invite code storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;

use crate::backend::Backend;

/// An invite code record returned by [`InviteStorage`] queries.
#[derive(Clone, Debug)]
pub struct InviteRecord {
    /// The alphanumeric invite code.
    pub code: String,
    /// When the code was generated.
    pub created_at: DateTime<Utc>,
    /// When the code will expire.
    pub expires_at: DateTime<Utc>,
    /// When the code was consumed (None if still active).
    pub used_at: Option<DateTime<Utc>>,
    /// ID of the user who was created using this code.
    pub used_by: Option<i64>,
}

/// Errors that can occur when consuming an invite code.
#[derive(Debug, Error)]
pub enum UseInviteError {
    /// The invite code does not exist.
    #[error("invite code not found")]
    NotFound,
    /// The invite code has passed its expiration date.
    #[error("invite code has expired")]
    Expired,
    /// The invite code has already been consumed.
    #[error("invite code has already been used")]
    AlreadyUsed,
}

/// Async operations on the `invites` table.
///
/// This trait manages the lifecycle of invite codes used for registration.
#[async_trait]
pub trait InviteStorage: Send + Sync {
    /// Generates and stores a new invite code.
    ///
    /// Returns the raw code string.
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String>;

    /// Marks an invite code as used by a specific user.
    ///
    /// # Errors
    ///
    /// Returns [`UseInviteError`] if the code is invalid, expired, or already used.
    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError>;

    /// Returns a list of all invite codes in the system.
    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}

/// Generic [`InviteStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (identical SQL across `SQLite` and Postgres),
/// so it is implemented once here; see ADR-0019.
pub struct InviteStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> InviteStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> InviteStorage for InviteStore<DB>
where
    DB: Backend,
    crate::helpers::InviteRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String> {
        let code = crate::auth::generate_token();
        let now = Utc::now();

        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)")
            .bind(code.as_str())
            .bind(now)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;

        Ok(code)
    }

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        let row = sqlx::query_as::<_, crate::helpers::InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UseInviteError::NotFound)?
        .ok_or(UseInviteError::NotFound)?;

        let record = crate::helpers::invite_record_from_row(row);

        if record.used_at.is_some() {
            return Err(UseInviteError::AlreadyUsed);
        }

        let now = Utc::now();
        if record.expires_at <= now {
            return Err(UseInviteError::Expired);
        }

        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(code)
            .execute(&mut *tx)
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        tx.commit().await.map_err(|_| UseInviteError::NotFound)?;

        Ok(())
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let rows = sqlx::query_as::<_, crate::helpers::InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by FROM invites",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(crate::helpers::invite_record_from_row)
            .collect())
    }
}

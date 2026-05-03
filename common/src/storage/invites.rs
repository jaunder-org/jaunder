use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// An invite code record returned by [`InviteStorage`] queries.
#[derive(Clone, Debug)]
pub struct InviteRecord {
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by: Option<i64>,
}

/// Errors that can occur when consuming an invite code.
#[derive(Debug, Error)]
pub enum UseInviteError {
    #[error("invite code not found")]
    NotFound,
    #[error("invite code has expired")]
    Expired,
    #[error("invite code has already been used")]
    AlreadyUsed,
}

/// Async operations on the `invites` table.
#[async_trait]
pub trait InviteStorage: Send + Sync {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String>;

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError>;

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>>;
}

//! Invite code storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

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

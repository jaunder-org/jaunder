//! Session and device token storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use common::username::Username;

/// A session record returned by [`SessionStorage`] queries.
#[derive(Clone, Debug)]
pub struct SessionRecord {
    /// SHA-256 hash of the session token.
    pub token_hash: String,
    /// ID of the user associated with this session.
    pub user_id: i64,
    /// Username at the time of session creation.
    pub username: Username,
    /// Optional user-provided label for the device/client (e.g., "Mobile App").
    pub label: Option<String>,
    /// When the session was first created.
    pub created_at: DateTime<Utc>,
    /// When the session was last used to authenticate a request.
    pub last_used_at: DateTime<Utc>,
}

/// Errors that can occur when authenticating a session token.
#[derive(Debug, Error)]
pub enum SessionAuthError {
    /// The token is malformed or invalid.
    #[error("invalid token")]
    InvalidToken,
    /// No active session matches the provided token.
    #[error("session not found")]
    SessionNotFound,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Async operations on the `sessions` table.
///
/// This trait manages the lifecycle of session tokens used for authenticating
/// web and API requests.
#[async_trait]
pub trait SessionStorage: Send + Sync {
    /// Creates a new session for a user.
    ///
    /// Returns the raw (un-hashed) token to be delivered to the client.
    async fn create_session(&self, user_id: i64, label: Option<&str>) -> sqlx::Result<String>;

    /// Validates a raw session token and returns the associated record.
    ///
    /// On success, updates the `last_used_at` timestamp for the session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionAuthError`] if the token is invalid or the session has
    /// been revoked.
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError>;

    /// Revokes a specific session by its token hash.
    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()>;

    /// Returns a list of all active sessions for a user.
    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>>;
}

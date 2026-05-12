use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::username::Username;

/// A session record returned by [`SessionStorage`] queries.
#[derive(Clone, Debug)]
pub struct SessionRecord {
    pub token_hash: String,
    pub user_id: i64,
    pub username: Username,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

/// Errors that can occur when authenticating a session token.
#[derive(Debug, Error)]
pub enum SessionAuthError {
    #[error("invalid token")]
    InvalidToken,
    #[error("session not found")]
    SessionNotFound,
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Async operations on the `sessions` table.
#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn create_session(&self, user_id: i64, label: Option<&str>) -> sqlx::Result<String>;

    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError>;

    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()>;

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>>;
}

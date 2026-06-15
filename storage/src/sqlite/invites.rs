use sqlx::Sqlite;

use crate::invites::InviteStore;

/// SQLite-backed invite code storage.
pub type SqliteInviteStorage = InviteStore<Sqlite>;

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;
    use crate::invites::InviteStorage;
    use crate::UseInviteError;

    #[tokio::test]
    async fn create_invite_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteInviteStorage::new(pool.clone());
        pool.close().await;
        let expires_at = chrono::Utc::now();
        let result = storage.create_invite(expires_at).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn use_invite_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteInviteStorage::new(pool.clone());
        pool.close().await;
        let result = storage.use_invite("code", 1).await;
        assert!(matches!(result, Err(UseInviteError::NotFound)));
    }

    #[tokio::test]
    async fn list_invites_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteInviteStorage::new(pool.clone());
        pool.close().await;
        let result = storage.list_invites().await;
        assert!(result.is_err());
    }
}

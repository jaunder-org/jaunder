use sqlx::Sqlite;

use crate::password::PasswordResetStore;

/// SQLite-backed password-reset token storage.
pub type SqlitePasswordResetStorage = PasswordResetStore<Sqlite>;

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;
    use crate::password::PasswordResetStorage;
    use crate::UsePasswordResetError;

    #[tokio::test]
    async fn create_password_reset_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqlitePasswordResetStorage::new(pool.clone());
        pool.close().await;
        let expires_at = chrono::Utc::now();
        let result = storage.create_password_reset(1, expires_at).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn use_password_reset_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqlitePasswordResetStorage::new(pool.clone());
        pool.close().await;
        let result = storage.use_password_reset("dGVzdA").await;
        assert!(matches!(result, Err(UsePasswordResetError::Internal(_))));
    }
}

use sqlx::Sqlite;

use crate::email::EmailVerificationStore;

/// SQLite-backed email verification token storage.
pub type SqliteEmailVerificationStorage = EmailVerificationStore<Sqlite>;

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;
    use crate::email::EmailVerificationStorage;
    use crate::UseEmailVerificationError;
    use email_address::EmailAddress;

    #[tokio::test]
    async fn create_email_verification_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteEmailVerificationStorage::new(pool.clone());
        pool.close().await;
        let expires_at = chrono::Utc::now();
        let email: EmailAddress = "test@example.com".parse().unwrap();
        let result = storage
            .create_email_verification(1, &email, expires_at)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn use_email_verification_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteEmailVerificationStorage::new(pool.clone());
        pool.close().await;
        let result = storage.use_email_verification("dGVzdA").await;
        assert!(matches!(result, Err(UseEmailVerificationError::NotFound)));
    }
}

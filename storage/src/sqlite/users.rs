use sqlx::Sqlite;

use crate::users::UserStore;

/// SQLite-backed user storage.
pub type SqliteUserStorage = UserStore<Sqlite>;

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;
    use crate::{UserAuthError, UserStorage};
    use common::password::Password;
    use common::username::Username;

    #[tokio::test]
    async fn authenticate_with_corrupted_hash_returns_internal_error() {
        let pool = sqlite_pool().await;
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator)
             VALUES ('alice', 'not-a-bcrypt-hash', datetime('now'), false)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let storage = SqliteUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[tokio::test]
    async fn authenticate_with_closed_pool_returns_internal_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteUserStorage::new(pool.clone());
        pool.close().await;
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        // §3.1a: the underlying sqlx::Error is preserved as a typed source
        // (not stringified), so the boundary can classify it.
        assert!(
            matches!(
                result,
                Err(UserAuthError::Internal(ref source))
                    if source.downcast_ref::<sqlx::Error>().is_some()
            ),
            "expected Internal carrying a sqlx::Error source"
        );
    }

    #[tokio::test]
    async fn authenticate_with_invalid_email_in_db_returns_internal_error() {
        let pool = sqlite_pool().await;
        let password: Password = "password123".parse().unwrap();
        let hash = crate::helpers::hash_password(password.clone())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (username, password_hash, email, created_at, is_operator)
             VALUES ('alice', $1, 'not-an-email', datetime('now'), false)",
        )
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();
        let storage = SqliteUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[tokio::test]
    async fn authenticate_with_blocked_update_returns_internal_error() {
        let pool = sqlite_pool().await;
        let password: Password = "password123".parse().unwrap();
        let hash = crate::helpers::hash_password(password.clone())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator)
             VALUES ('alice', $1, datetime('now'), false)",
        )
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER block_auth_update
             BEFORE UPDATE OF last_authenticated_at ON users
             BEGIN SELECT RAISE(FAIL, 'blocked'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let storage = SqliteUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[tokio::test]
    async fn create_user_with_hash_error_returns_internal_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let password: Password = "force-hash-error-for-test-coverage".parse().unwrap();
        let result = storage.create_user(&username, &password, None, false).await;
        assert!(matches!(result, Err(crate::CreateUserError::Internal(_))));
    }

    #[tokio::test]
    async fn set_password_updates_stored_credential() {
        let pool = sqlite_pool().await;
        let storage = SqliteUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let old_password: Password = "oldpassword123".parse().unwrap();
        let new_password: Password = "newpassword456".parse().unwrap();
        storage
            .create_user(&username, &old_password, None, false)
            .await
            .unwrap();
        let user = storage
            .get_user_by_username(&username)
            .await
            .unwrap()
            .unwrap();
        storage
            .set_password(user.user_id, &new_password)
            .await
            .unwrap();
        assert!(matches!(
            storage.authenticate(&username, &old_password).await,
            Err(UserAuthError::InvalidCredentials)
        ));
        assert!(storage.authenticate(&username, &new_password).await.is_ok());
    }

    #[tokio::test]
    async fn authenticate_with_verify_error_returns_internal_error() {
        let pool = sqlite_pool().await;
        let password: Password = "force-verify-error-for-test-coverage".parse().unwrap();
        let storage = SqliteUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let normal_password: Password = "password123".parse().unwrap();
        storage
            .create_user(&username, &normal_password, None, false)
            .await
            .unwrap();

        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }
}

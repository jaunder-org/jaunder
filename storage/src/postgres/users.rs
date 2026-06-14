use async_trait::async_trait;
use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use sqlx::PgPool;
use tracing::Instrument;

use crate::{CreateUserError, ProfileUpdate, UserAuthError, UserRecord, UserStorage};
use common::password::Password;
use common::username::Username;

use crate::helpers::{user_record_from_row, UserRow};

pub struct PostgresUserStorage {
    pool: PgPool,
}

impl PostgresUserStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserStorage for PostgresUserStorage {
    #[tracing::instrument(
        name = "storage.postgres.user.create_user",
        skip(self, password, display_name),
        fields(username = %username.as_str())
    )]
    async fn create_user(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
    ) -> Result<i64, CreateUserError> {
        let password_hash = crate::helpers::hash_password(password.clone())
            .instrument(tracing::info_span!(
                "storage.postgres.user.create_user.hash_password"
            ))
            .await
            .map_err(|e| CreateUserError::Internal(sqlx::Error::Io(e)))?;

        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING user_id",
        )
        .bind(username.as_str())
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .bind(is_operator)
        .fetch_one(&self.pool)
        .instrument(tracing::info_span!(
            "storage.postgres.user.create_user.insert_user_row"
        ))
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(CreateUserError::UsernameTaken)
            }
            Err(error) => Err(CreateUserError::Internal(error)),
        }
    }

    #[tracing::instrument(
        name = "storage.postgres.user.authenticate",
        skip(self, password),
        fields(username = %username.as_str())
    )]
    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError> {
        let row = sqlx::query_as::<
            _,
            (
                i64,
                String,
                Option<String>,
                Option<String>,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                String,
                Option<String>,
                bool,
                bool,
            ),
        >(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    password_hash, email, email_verified, is_operator
             FROM users WHERE username = $1",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .instrument(tracing::info_span!(
            "storage.postgres.user.authenticate.lookup_user"
        ))
        .await
        .map_err(|e| UserAuthError::Internal(Box::new(e)))?;

        let Some((
            user_id,
            username,
            display_name,
            bio,
            created_at,
            _last_authenticated_at,
            hash,
            email,
            email_verified,
            is_operator,
        )) = row
        else {
            // Equalize timing with the present-user path to avoid a username
            // enumeration oracle (§2.1): perform a dummy Argon2 verification
            // before rejecting. The result is intentionally discarded.
            let _ = crate::helpers::verify_password(
                password.clone(),
                crate::helpers::dummy_password_hash().to_string(),
            )
            .await;
            return Err(UserAuthError::InvalidCredentials);
        };

        let valid = crate::helpers::verify_password(password.clone(), hash)
            .instrument(tracing::info_span!(
                "storage.postgres.user.authenticate.verify_password"
            ))
            .await
            .map_err(|e| UserAuthError::Internal(Box::new(e)))?;

        if !valid {
            return Err(UserAuthError::InvalidCredentials);
        }

        let now = Utc::now();

        sqlx::query("UPDATE users SET last_authenticated_at = $1 WHERE user_id = $2")
            .bind(now)
            .bind(user_id)
            .execute(&self.pool)
            .instrument(tracing::info_span!(
                "storage.postgres.user.authenticate.update_last_authenticated_at"
            ))
            .await
            .map_err(|e| UserAuthError::Internal(Box::new(e)))?;

        crate::helpers::build_user_record((
            user_id,
            username,
            display_name,
            bio,
            created_at,
            Some(now),
            email,
            email_verified,
            is_operator,
        ))
        .map_err(|e| UserAuthError::Internal(Box::new(e)))
    }

    async fn get_user(&self, user_id: i64) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    email, email_verified, is_operator
             FROM users WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row).transpose()?)
    }

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    email, email_verified, is_operator
             FROM users WHERE username = $1",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row).transpose()?)
    }

    async fn update_profile(&self, user_id: i64, update: &ProfileUpdate<'_>) -> sqlx::Result<()> {
        sqlx::query("UPDATE users SET display_name = $1, bio = $2 WHERE user_id = $3")
            .bind(update.display_name)
            .bind(update.bio)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_email(
        &self,
        user_id: i64,
        email: Option<&EmailAddress>,
        verified: bool,
    ) -> sqlx::Result<()> {
        sqlx::query("UPDATE users SET email = $1, email_verified = $2 WHERE user_id = $3")
            .bind(email.map(EmailAddress::as_str))
            .bind(verified)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_password(&self, user_id: i64, new_password: &Password) -> sqlx::Result<()> {
        let password_hash = crate::helpers::hash_password(new_password.clone())
            .await
            .map_err(sqlx::Error::Io)?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::postgres_pool;
    use super::*;

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn authenticate_with_corrupted_hash_returns_internal_error() {
        let pool = postgres_pool().await;
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator)
             VALUES ('alice', 'not-a-bcrypt-hash', now(), false)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let storage = PostgresUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn authenticate_with_closed_pool_returns_internal_error() {
        let pool = postgres_pool().await;
        let storage = PostgresUserStorage::new(pool.clone());
        pool.close().await;
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn authenticate_with_invalid_email_in_db_returns_internal_error() {
        let pool = postgres_pool().await;
        let password: Password = "password123".parse().unwrap();
        let hash = crate::helpers::hash_password(password.clone())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (username, password_hash, email, created_at, is_operator)
             VALUES ('alice', $1, 'not-an-email', now(), false)",
        )
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();
        let storage = PostgresUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn authenticate_with_blocked_update_returns_internal_error() {
        let pool = postgres_pool().await;
        let password: Password = "password123".parse().unwrap();
        let hash = crate::helpers::hash_password(password.clone())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator)
             VALUES ('alice', $1, now(), false)",
        )
        .bind(&hash)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE OR REPLACE FUNCTION raise_block() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'blocked'; END; $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER block_auth_update
             BEFORE UPDATE OF last_authenticated_at ON users
             FOR EACH ROW EXECUTE FUNCTION raise_block()",
        )
        .execute(&pool)
        .await
        .unwrap();
        let storage = PostgresUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let result = storage.authenticate(&username, &password).await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[tokio::test]
    async fn create_user_with_hash_error_returns_internal_error() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost:1/jaunder").unwrap();
        let storage = PostgresUserStorage::new(pool);
        let username: Username = "alice".parse().unwrap();
        let password: Password = "force-hash-error-for-test-coverage".parse().unwrap();
        let result = storage.create_user(&username, &password, None, false).await;
        assert!(matches!(result, Err(CreateUserError::Internal(_))));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn authenticate_with_verify_error_returns_internal_error() {
        let pool = postgres_pool().await;
        let password: Password = "force-verify-error-for-test-coverage".parse().unwrap();
        let storage = PostgresUserStorage::new(pool);
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

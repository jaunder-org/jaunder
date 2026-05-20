use async_trait::async_trait;
use chrono::DateTime;
use sqlx::PgPool;

use crate::helpers::generate_hashed_token;
use crate::helpers::password_reset_claim_error;
use crate::{PasswordResetStorage, UsePasswordResetError};

pub struct PostgresPasswordResetStorage {
    pool: PgPool,
}

impl PostgresPasswordResetStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PasswordResetStorage for PostgresPasswordResetStorage {
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<chrono::Utc>,
    ) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = chrono::Utc::now();

        sqlx::query(
            "INSERT INTO password_resets (token_hash, user_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    async fn use_password_reset(&self, raw_token: &str) -> Result<i64, UsePasswordResetError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| UsePasswordResetError::NotFound)?;

        let now = chrono::Utc::now();

        let claimed = sqlx::query_as::<_, (i64,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((user_id,)) = claimed {
            return Ok(user_id);
        }

        let row = sqlx::query_as::<_, (Option<DateTime<chrono::Utc>>, DateTime<chrono::Utc>)>(
            "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Err(password_reset_claim_error(row))
    }
}

#[cfg(test)]
mod tests {
    use super::super::postgres_pool;
    use super::*;

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn create_password_reset_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresPasswordResetStorage::new(pool.clone());
        pool.close().await;
        let expires_at = chrono::Utc::now();
        let result = storage.create_password_reset(1, expires_at).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn use_password_reset_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresPasswordResetStorage::new(pool.clone());
        pool.close().await;
        let result = storage.use_password_reset("dGVzdA").await;
        assert!(matches!(result, Err(UsePasswordResetError::Internal(_))));
    }
}

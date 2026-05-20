use async_trait::async_trait;
use chrono::DateTime;
use sqlx::SqlitePool;

use crate::helpers::{email_verification_claim_error, generate_hashed_token};
use crate::{EmailVerificationStorage, UseEmailVerificationError};

pub struct SqliteEmailVerificationStorage {
    pool: SqlitePool,
}

impl SqliteEmailVerificationStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EmailVerificationStorage for SqliteEmailVerificationStorage {
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &str,
        expires_at: DateTime<chrono::Utc>,
    ) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = chrono::Utc::now();

        let mut tx = self.pool.begin().await?;

        // Supersede any existing pending token for this user by setting its
        // expires_at to its created_at, making it appear immediately expired.
        sqlx::query(
            "UPDATE email_verifications
             SET expires_at = created_at
             WHERE user_id = $1 AND used_at IS NULL AND expires_at > $2",
        )
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO email_verifications
             (token_hash, user_id, email, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(email)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(raw_token)
    }

    async fn use_email_verification(
        &self,
        raw_token: &str,
    ) -> Result<(i64, String), UseEmailVerificationError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| UseEmailVerificationError::NotFound)?;

        let now = chrono::Utc::now();

        // Atomically claim the token: the UPDATE succeeds only when the token
        // exists, has not yet been used, and has not expired.  This single
        // statement is the "claim" — no separate read is needed first, so two
        // concurrent requests cannot both succeed.  RETURNING gives us the
        // data we need without a second round-trip.
        let claimed = sqlx::query_as::<_, (i64, String)>(
            "UPDATE email_verifications SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id, email",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseEmailVerificationError::NotFound)?;

        if let Some((user_id, email)) = claimed {
            return Ok((user_id, email));
        }

        // Zero rows affected — inspect the row to return the right error.
        let row = sqlx::query_as::<_, (Option<DateTime<chrono::Utc>>, DateTime<chrono::Utc>)>(
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| UseEmailVerificationError::NotFound)?;

        Err(email_verification_claim_error(row))
    }
}

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;

    #[tokio::test]
    async fn create_email_verification_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteEmailVerificationStorage::new(pool.clone());
        pool.close().await;
        let expires_at = chrono::Utc::now();
        let result = storage
            .create_email_verification(1, "test@example.com", expires_at)
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

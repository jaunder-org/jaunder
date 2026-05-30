use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

use crate::helpers::{generate_hashed_token, session_record_from_row, SessionRow};
use crate::{SessionAuthError, SessionRecord, SessionStorage};

pub struct PostgresSessionStorage {
    pool: PgPool,
}

impl PostgresSessionStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStorage for PostgresSessionStorage {
    #[tracing::instrument(
        name = "storage.postgres.session.create",
        skip(self, label),
        fields(user_id)
    )]
    async fn create_session(&self, user_id: i64, label: &str) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, label, created_at, last_used_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(label)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    #[tracing::instrument(name = "storage.postgres.session.authenticate", skip(self, raw_token))]
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let now = Utc::now();

        // Perform an atomic update and read in a single statement.
        // PostgreSQL's data-modifying CTEs (WITH UPDATE ...) are used
        // here to achievement atomicity while joining the results with
        // another table.
        let row = sqlx::query_as::<_, SessionRow>(
            "WITH updated AS (
                UPDATE sessions
                SET last_used_at = $1
                WHERE token_hash = $2
                RETURNING token_hash, user_id, label, created_at, last_used_at
             )
             SELECT updated.token_hash, updated.user_id, u.username, updated.label, updated.created_at, updated.last_used_at
             FROM updated
             JOIN users u ON updated.user_id = u.user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(SessionAuthError::SessionNotFound)?;

        let record = session_record_from_row(row)?;
        Ok(record)
    }

    #[tracing::instrument(name = "storage.postgres.session.revoke", skip(self, token_hash))]
    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(session_record_from_row).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::postgres_pool;
    use super::*;

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn authenticate_with_closed_pool_returns_internal_error() {
        let pool = postgres_pool().await;
        let storage = PostgresSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.authenticate("dGVzdA").await;
        assert!(matches!(result, Err(SessionAuthError::Internal(_))));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn create_session_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.create_session(1, "device").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn revoke_session_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.revoke_session("token-hash").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL test VM"]
    async fn list_sessions_with_closed_pool_returns_error() {
        let pool = postgres_pool().await;
        let storage = PostgresSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.list_sessions(1).await;
        assert!(result.is_err());
    }
}

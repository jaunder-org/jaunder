use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;

use crate::helpers::{generate_hashed_token, session_record_from_row, SessionRow};
use crate::{SessionAuthError, SessionRecord, SessionStorage};

pub struct SqliteSessionStorage {
    pool: SqlitePool,
}

impl SqliteSessionStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStorage for SqliteSessionStorage {
    #[tracing::instrument(
        name = "storage.sqlite.session.create",
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

    #[tracing::instrument(name = "storage.sqlite.session.authenticate", skip(self, raw_token))]
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;

        let now = Utc::now();

        // Perform an atomic update and read in a single statement. This
        // avoids the need for a multi-statement transaction, which can
        // cause SQLITE_BUSY contention in high-concurrency environments.
        //
        // Note: SQLite's RETURNING clause is used with a correlated subquery
        // Split the update and select into two operations to avoid the
        // subquery overhead in the RETURNING clause and potentially
        // reduce disk I/O contention.
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
            .bind(now)
            .bind(&token_hash)
            .execute(&mut *tx)
            .await?;

        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SessionAuthError::SessionNotFound)?;

        tx.commit().await?;

        let record = session_record_from_row(row)?;
        Ok(record)
    }

    #[tracing::instrument(name = "storage.sqlite.session.revoke", skip(self, token_hash))]
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
    use super::super::sqlite_pool;
    use super::*;

    #[tokio::test]
    async fn authenticate_with_closed_pool_returns_internal_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.authenticate("dGVzdA").await;
        assert!(matches!(result, Err(SessionAuthError::Internal(_))));
    }

    #[tokio::test]
    async fn create_session_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.create_session(1, "device").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn revoke_session_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.revoke_session("token-hash").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_sessions_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.list_sessions(1).await;
        assert!(result.is_err());
    }
}

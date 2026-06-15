use async_trait::async_trait;
use sqlx::{Pool, Sqlite};

use crate::helpers::SessionRow;
use crate::sessions::{SessionDialect, SessionStore};

/// SQLite-backed session storage.
pub type SqliteSessionStorage = SessionStore<Sqlite>;

#[async_trait]
impl SessionDialect for Sqlite {
    async fn touch_and_load(
        pool: &Pool<Sqlite>,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> sqlx::Result<Option<SessionRow>> {
        // Two statements in one tx: SQLite's RETURNING with a correlated
        // subquery causes SQLITE_BUSY under concurrency, so update then select.
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
            .bind(now)
            .bind(token_hash)
            .execute(&mut *tx)
            .await?;

        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;
    use crate::SessionStorage as _;

    #[tokio::test]
    async fn authenticate_with_closed_pool_returns_internal_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteSessionStorage::new(pool.clone());
        pool.close().await;
        let result = storage.authenticate("dGVzdA").await;
        assert!(matches!(result, Err(crate::SessionAuthError::Internal(_))));
    }
}

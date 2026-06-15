use async_trait::async_trait;
use sqlx::{Pool, Postgres};

use crate::helpers::SessionRow;
use crate::sessions::{SessionDialect, SessionStore};

/// Postgres-backed session storage.
pub type PostgresSessionStorage = SessionStore<Postgres>;

#[async_trait]
impl SessionDialect for Postgres {
    async fn touch_and_load(
        pool: &Pool<Postgres>,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> sqlx::Result<Option<SessionRow>> {
        // Postgres can update-and-join atomically with a data-modifying CTE.
        sqlx::query_as::<_, SessionRow>(
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
        .bind(token_hash)
        .fetch_optional(pool)
        .await
    }
}

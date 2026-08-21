use async_trait::async_trait;
use sqlx::{Pool, Sqlite};

use common::token::TokenHash;

use crate::helpers::SessionRow;
use crate::sessions::{SessionDialect, SessionStore};

/// SQLite-backed session storage.
pub type SqliteSessionStorage = SessionStore<Sqlite>;

#[async_trait]
impl SessionDialect for Sqlite {
    async fn touch_and_load(
        pool: &Pool<Sqlite>,
        token_hash: &TokenHash,
        now: chrono::DateTime<chrono::Utc>,
        stale_before: chrono::DateTime<chrono::Utc>,
    ) -> sqlx::Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let (_, _, _, _, _, last_used_at) = &row;
        if *last_used_at >= stale_before {
            return Ok(Some(row));
        }

        sqlx::query(
            "UPDATE sessions
             SET last_used_at = $1
             WHERE token_hash = $2 AND last_used_at < $3",
        )
        .bind(now)
        .bind(token_hash)
        .bind(stale_before)
        .execute(pool)
        .await?;

        sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(pool)
        .await
    }
}

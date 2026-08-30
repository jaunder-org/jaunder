use async_trait::async_trait;
use sqlx::Postgres;

use common::time::UtcInstant;
use common::token::TokenHash;

use crate::WriteTransaction;
use crate::helpers::SessionRow;
use crate::sessions::{SessionDialect, SessionStore};

/// Postgres-backed session storage.
pub type PostgresSessionStorage = SessionStore<Postgres>;

#[async_trait]
impl SessionDialect for Postgres {
    async fn touch_and_load(
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
        now: UtcInstant,
        stale_before: UtcInstant,
    ) -> sqlx::Result<Option<SessionRow>> {
        let connection = <Postgres as crate::backend::Backend>::write_connection(transaction)?;
        let updated = sqlx::query_as::<_, SessionRow>(
            "WITH updated AS (
                 UPDATE sessions
                 SET last_used_at = $1
                 WHERE token_hash = $2 AND last_used_at < $3
                 RETURNING token_hash, user_id, label, created_at, last_used_at
             )
             SELECT updated.token_hash, updated.user_id, u.username, updated.label,
                    updated.created_at, updated.last_used_at
             FROM updated
             JOIN users u ON updated.user_id = u.user_id",
        )
        .bind(now)
        .bind(token_hash)
        .bind(stale_before)
        .fetch_optional(&mut *connection)
        .await?;

        if updated.is_some() {
            return Ok(updated);
        }

        sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&mut *connection)
        .await
    }
}

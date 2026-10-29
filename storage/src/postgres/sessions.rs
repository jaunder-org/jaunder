use async_trait::async_trait;
use sqlx::Postgres;

use common::token::TokenHash;

use crate::WriteTransaction;
use crate::helpers::SessionRow;
use crate::sessions::{
    SessionClock, SessionDialect, SessionStore, session_idle_cutoff, session_touch_cutoff,
};
use crate::sql::QueryStorageExt;

/// Postgres-backed session storage.
pub type PostgresSessionStorage = SessionStore<Postgres>;

#[async_trait]
impl SessionDialect for Postgres {
    async fn touch_and_load(
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
        clock: &SessionClock,
    ) -> sqlx::Result<Option<(SessionRow, bool)>> {
        let connection = <Postgres as crate::backend::Backend>::write_connection(transaction)?;
        // Lock even a throttled read until the enclosing commit. Revocation,
        // expiry and touch therefore share one row-level serialization order.
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at, s.kind
             FROM sessions s JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1
             FOR UPDATE OF s",
        )
        .bind_storage(token_hash)
        .fetch_optional(&mut *connection)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let now = clock.now();
        let stale_before = session_touch_cutoff(now);
        let expired_at_or_before = session_idle_cutoff(now);
        // Evaluate expiry only after locking: an uncommitted eligible touch may
        // advance an otherwise-expired snapshot while this statement waits.
        if row.kind() == crate::SessionKind::Browser && row.last_used_at() <= expired_at_or_before {
            return Ok(None);
        }
        if row.last_used_at() > stale_before {
            return Ok(Some((row, false)));
        }
        sqlx::query_as::<_, SessionRow>(
            "WITH updated AS (
                 UPDATE sessions SET last_used_at = $1
                 WHERE token_hash = $2 AND last_used_at <= $3
                   AND (kind != 'browser' OR last_used_at > $4)
                 RETURNING token_hash, user_id, label, created_at, last_used_at, kind
             )
             SELECT updated.token_hash, updated.user_id, u.username, updated.label,
                    updated.created_at, updated.last_used_at, updated.kind
             FROM updated JOIN users u ON updated.user_id = u.user_id",
        )
        .bind_storage(now)
        .bind_storage(token_hash)
        .bind_storage(stale_before)
        .bind_storage(expired_at_or_before)
        .fetch_optional(&mut *connection)
        .await
        .map(|row| row.map(|row| (row, true)))
    }
}

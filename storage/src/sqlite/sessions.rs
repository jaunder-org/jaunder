use async_trait::async_trait;
use sqlx::Sqlite;

use common::token::TokenHash;

use crate::WriteTransaction;
use crate::helpers::SessionRow;
use crate::sessions::{
    SessionClock, SessionDialect, SessionStore, session_idle_cutoff, session_touch_cutoff,
};
use crate::sql::QueryStorageExt;

/// SQLite-backed session storage.
pub type SqliteSessionStorage = SessionStore<Sqlite>;

#[async_trait]
impl SessionDialect for Sqlite {
    async fn touch_and_load(
        transaction: &mut WriteTransaction,
        token_hash: &TokenHash,
        clock: &SessionClock,
    ) -> sqlx::Result<Option<(SessionRow, bool)>> {
        let connection = <Sqlite as crate::backend::Backend>::write_connection(transaction)?;
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at, s.kind
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind_storage(token_hash)
        .fetch_optional(&mut *connection)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        // WriteScope acquired BEGIN IMMEDIATE before this read. Sample time
        // only after that potentially long wait, never at request entry.
        let now = clock.now();
        let stale_before = session_touch_cutoff(now);
        let expired_at_or_before = session_idle_cutoff(now);
        if row.kind() == crate::SessionKind::Browser && row.last_used_at() <= expired_at_or_before {
            return Ok(None);
        }
        if row.last_used_at() > stale_before {
            return Ok(Some((row, false)));
        }

        sqlx::query(
            "UPDATE sessions
             SET last_used_at = $1
             WHERE token_hash = $2 AND last_used_at <= $3
               AND (kind != 'browser' OR last_used_at > $4)",
        )
        .bind_storage(now)
        .bind_storage(token_hash)
        .bind_storage(stale_before)
        .bind_storage(expired_at_or_before)
        .execute(&mut *connection)
        .await?;

        sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at, s.kind
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind_storage(token_hash)
        .fetch_optional(&mut *connection)
        .await
        .map(|row| row.map(|row| (row, true)))
    }
}

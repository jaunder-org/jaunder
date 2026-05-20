use async_trait::async_trait;
use chrono::DateTime;
use sqlx::SqlitePool;

use crate::helpers::{invite_record_from_row, InviteRow};
use crate::{InviteRecord, InviteStorage, UseInviteError};

pub struct SqliteInviteStorage {
    pool: SqlitePool,
}

impl SqliteInviteStorage {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteStorage for SqliteInviteStorage {
    async fn create_invite(&self, expires_at: DateTime<chrono::Utc>) -> sqlx::Result<String> {
        let code = crate::auth::generate_token();
        let now = chrono::Utc::now();

        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)")
            .bind(&code)
            .bind(now)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;

        Ok(code)
    }

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        let row = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UseInviteError::NotFound)?
        .ok_or(UseInviteError::NotFound)?;

        let record = invite_record_from_row(row);

        if record.used_at.is_some() {
            return Err(UseInviteError::AlreadyUsed);
        }

        let now = chrono::Utc::now();
        if record.expires_at <= now {
            return Err(UseInviteError::Expired);
        }

        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(code)
            .execute(&mut *tx)
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        tx.commit().await.map_err(|_| UseInviteError::NotFound)?;

        Ok(())
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let rows = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by FROM invites",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(invite_record_from_row).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::sqlite_pool;
    use super::*;

    #[tokio::test]
    async fn create_invite_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteInviteStorage::new(pool.clone());
        pool.close().await;
        let expires_at = chrono::Utc::now();
        let result = storage.create_invite(expires_at).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn use_invite_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteInviteStorage::new(pool.clone());
        pool.close().await;
        let result = storage.use_invite("code", 1).await;
        assert!(matches!(result, Err(UseInviteError::NotFound)));
    }

    #[tokio::test]
    async fn list_invites_with_closed_pool_returns_error() {
        let pool = sqlite_pool().await;
        let storage = SqliteInviteStorage::new(pool.clone());
        pool.close().await;
        let result = storage.list_invites().await;
        assert!(result.is_err());
    }
}

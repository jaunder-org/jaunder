use async_trait::async_trait;
use sqlx::PgPool;

use crate::UserConfigStorage;

pub struct PostgresUserConfigStorage {
    pool: PgPool,
}

impl PostgresUserConfigStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserConfigStorage for PostgresUserConfigStorage {
    #[tracing::instrument(name = "storage.postgres.user_config.get", skip(self))]
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT value FROM user_config WHERE user_id = $1 AND key = $2",
        )
        .bind(user_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(v,)| v))
    }

    #[tracing::instrument(name = "storage.postgres.user_config.set", skip(self))]
    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO user_config (user_id, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(user_id)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(name = "storage.postgres.user_config.delete", skip(self))]
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM user_config WHERE user_id = $1 AND key = $2")
            .bind(user_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

use async_trait::async_trait;
use sqlx::PgPool;

use crate::SiteConfigStorage;

pub struct PostgresSiteConfigStorage {
    pool: PgPool,
}

impl PostgresSiteConfigStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SiteConfigStorage for PostgresSiteConfigStorage {
    async fn get(&self, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>("SELECT value FROM site_config WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(value,)| value))
    }

    async fn set(&self, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO site_config (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

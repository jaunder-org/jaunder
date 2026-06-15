//! Per-user preference storage.

use crate::backend::Backend;
use crate::posts::PostFormat;
use async_trait::async_trait;
use sqlx::{Database, Pool};

/// Async operations on the `user_config` key-value table.
///
/// This trait manages individual user preferences and settings, which are
/// separate from site-wide configuration.
#[async_trait]
pub trait UserConfigStorage: Send + Sync {
    /// Returns a user's configuration value for a specific key.
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>>;

    /// Sets or updates a user's configuration value.
    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()>;

    /// Deletes a specific configuration key for a user.
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()>;
}

/// Key for a user's media cache policy (e.g., whether to cache remote content).
pub const USER_MEDIA_CACHE_POLICY_KEY: &str = "media.cache_policy";

/// Key for a user's default post format preference.
pub const DEFAULT_POST_FORMAT_KEY: &str = "posts.default_format";

/// Reads a user's default post format preference, falling back to `Html` when
/// unset or unparseable.
///
/// # Errors
///
/// Returns a database error if the query fails.
pub async fn get_default_post_format(
    config: &dyn UserConfigStorage,
    user_id: i64,
) -> sqlx::Result<PostFormat> {
    let raw = config.get(user_id, DEFAULT_POST_FORMAT_KEY).await?;
    Ok(raw
        .as_deref()
        .and_then(|s| s.parse::<PostFormat>().ok())
        .unwrap_or(PostFormat::Html))
}

/// Sets a user's default post format preference.
///
/// # Errors
///
/// Returns a database error if the query fails.
pub async fn set_default_post_format(
    config: &dyn UserConfigStorage,
    user_id: i64,
    format: PostFormat,
) -> sqlx::Result<()> {
    config
        .set(user_id, DEFAULT_POST_FORMAT_KEY, &format.to_string())
        .await
}

/// Generic [`UserConfigStorage`] backed by any [`Backend`] database.
///
/// `UserConfigStorage` has no per-backend divergence (the upsert uses the shared
/// `ON CONFLICT ... DO UPDATE` form), so there is no dialect trait — the
/// implementation is written once here. See ADR-0019.
pub struct UserConfigStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> UserConfigStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> UserConfigStorage for UserConfigStore<DB>
where
    DB: Backend,
    // Restated from `Backend` (supertrait where-clauses don't propagate; ADR-0019),
    // plus the `(String,)` row decode for `get` and the query-arguments bound.
    (String,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.user_config.get",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
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

    #[tracing::instrument(
        name = "storage.user_config.set",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
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

    #[tracing::instrument(
        name = "storage.user_config.delete",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM user_config WHERE user_id = $1 AND key = $2")
            .bind(user_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqliteUserConfigStorage;

    #[tokio::test]
    async fn get_default_post_format_unset_returns_html() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../storage/migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();

        // Create a test user
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("testuser")
        .bind("hash")
        .bind(chrono::Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        let config = SqliteUserConfigStorage::new(pool);
        let result = get_default_post_format(&config, 1).await.unwrap();
        assert_eq!(result, PostFormat::Html);
    }

    #[tokio::test]
    async fn set_and_get_default_post_format_markdown() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../storage/migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();

        // Create a test user
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("testuser")
        .bind("hash")
        .bind(chrono::Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        let config = SqliteUserConfigStorage::new(pool);
        set_default_post_format(&config, 1, PostFormat::Markdown)
            .await
            .unwrap();
        let result = get_default_post_format(&config, 1).await.unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }

    #[tokio::test]
    async fn set_and_get_default_post_format_org() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../storage/migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();

        // Create a test user
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("testuser")
        .bind("hash")
        .bind(chrono::Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        let config = SqliteUserConfigStorage::new(pool);
        set_default_post_format(&config, 1, PostFormat::Org)
            .await
            .unwrap();
        let result = get_default_post_format(&config, 1).await.unwrap();
        assert_eq!(result, PostFormat::Org);
    }

    #[tokio::test]
    async fn get_default_post_format_invalid_string_returns_html() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("../storage/migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();

        // Create a test user
        sqlx::query(
            "INSERT INTO users (username, password_hash, created_at, is_operator) VALUES (?, ?, ?, ?)",
        )
        .bind("testuser")
        .bind("hash")
        .bind(chrono::Utc::now())
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        // Manually insert garbage value
        sqlx::query("INSERT INTO user_config (user_id, key, value) VALUES (?, ?, ?)")
            .bind(1)
            .bind(DEFAULT_POST_FORMAT_KEY)
            .bind("garbage")
            .execute(&pool)
            .await
            .unwrap();

        let config = SqliteUserConfigStorage::new(pool);
        let result = get_default_post_format(&config, 1).await.unwrap();
        assert_eq!(result, PostFormat::Html);
    }
}

//! Per-user preference storage.

use crate::posts::PostFormat;
use async_trait::async_trait;

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

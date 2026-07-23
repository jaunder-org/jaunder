//! Per-user preference storage.

use crate::backend::Backend;
use crate::posts::PostFormat;
use async_trait::async_trait;
use common::ids::UserId;
use sqlx::{Database, Pool};

/// Async operations on the `user_config` key-value table.
///
/// This trait manages individual user preferences and settings, which are
/// separate from site-wide configuration.
#[async_trait]
pub trait UserConfigStorage: Send + Sync {
    /// Returns a user's configuration value for a specific key.
    async fn get(&self, user_id: UserId, key: &str) -> sqlx::Result<Option<String>>;

    /// Sets or updates a user's configuration value.
    async fn set(&self, user_id: UserId, key: &str, value: &str) -> sqlx::Result<()>;

    /// Deletes a specific configuration key for a user.
    async fn delete(&self, user_id: UserId, key: &str) -> sqlx::Result<()>;
}

/// Key for a user's media cache policy (e.g., whether to cache remote content).
pub const USER_MEDIA_CACHE_POLICY_KEY: &str = "media.cache_policy";

/// Key for a user's default post format preference.
pub const DEFAULT_POST_FORMAT_KEY: &str = "posts.default_format";

/// Reads a user's default post format preference, falling back to `Markdown`
/// when unset or unparseable.
///
/// The fallback is a *user-authoring* format: `Html` is renderer-internal (#445)
/// — it carries no editor message and is not offered by any format picker — so an
/// unset/garbage preference resolves to `Markdown`, the first offered format.
///
/// # Errors
///
/// Returns a database error if the query fails.
pub async fn get_default_post_format(
    config: &dyn UserConfigStorage,
    user_id: UserId,
) -> sqlx::Result<PostFormat> {
    let raw = config.get(user_id, DEFAULT_POST_FORMAT_KEY).await?;
    Ok(raw
        .as_deref()
        .and_then(|s| s.parse::<PostFormat>().ok())
        .unwrap_or(PostFormat::Markdown))
}

/// Sets a user's default post format preference.
///
/// # Errors
///
/// Returns a database error if the query fails.
pub async fn set_default_post_format(
    config: &dyn UserConfigStorage,
    user_id: UserId,
    format: PostFormat,
) -> sqlx::Result<()> {
    config
        .set(user_id, DEFAULT_POST_FORMAT_KEY, format.as_ref())
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
    async fn get(&self, user_id: UserId, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT value FROM user_config WHERE user_id = $1 AND key = $2",
        )
        .bind(i64::from(user_id))
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
    async fn set(&self, user_id: UserId, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO user_config (user_id, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(i64::from(user_id))
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
    async fn delete(&self, user_id: UserId, key: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM user_config WHERE user_id = $1 AND key = $2")
            .bind(i64::from(user_id))
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, seed_user, Backend};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn get_default_post_format_unset_returns_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let config = &*env.state.user_config;
        let result = get_default_post_format(config, user_id).await.unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_and_get_default_post_format_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let config = &*env.state.user_config;
        set_default_post_format(config, user_id, PostFormat::Markdown)
            .await
            .unwrap();
        let result = get_default_post_format(config, user_id).await.unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_and_get_default_post_format_org(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let config = &*env.state.user_config;
        set_default_post_format(config, user_id, PostFormat::Org)
            .await
            .unwrap();
        let result = get_default_post_format(config, user_id).await.unwrap();
        assert_eq!(result, PostFormat::Org);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_default_post_format_invalid_string_returns_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;
        let config = &*env.state.user_config;

        // Store a garbage value through the storage handle.
        config
            .set(user_id, DEFAULT_POST_FORMAT_KEY, "garbage")
            .await
            .unwrap();

        let result = get_default_post_format(config, user_id).await.unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }
}

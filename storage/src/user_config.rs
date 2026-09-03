//! Per-user preference storage.

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::posts::models::PostFormat;
use crate::sql::QueryStorageExt;
use async_trait::async_trait;
use common::ids::UserId;
use common::theme::Theme;
use sqlx::{Database, Encode, Executor, Pool, Result, Type};

use host::config_key::UserConfigKey;

/// A user-config value preserved exactly until its key-specific read policy parses it.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct StoredUserConfigValue(String);

impl StoredUserConfigValue {
    fn into_inner(self) -> String {
        self.0
    }
}

/// Async operations on the `user_config` key-value table.
///
/// This trait manages individual user preferences and settings, which are
/// separate from site-wide configuration.
#[async_trait]
pub trait UserConfigStorage: Send + Sync {
    /// Returns a user's configuration value for a specific key.
    async fn get(&self, user_id: UserId, key: UserConfigKey) -> Result<Option<String>>;

    /// Sets or updates a user's configuration value.
    async fn set(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        key: UserConfigKey,
        value: &str,
    ) -> Result<()>;

    /// Deletes a specific configuration key for a user.
    async fn delete(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        key: UserConfigKey,
    ) -> Result<()>;
}

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
) -> Result<PostFormat> {
    let raw = config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await?;
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
    transaction: &mut WriteTransaction,
    user_id: UserId,
    format: PostFormat,
) -> Result<()> {
    config
        .set(
            transaction,
            user_id,
            UserConfigKey::DefaultPostFormat,
            format.as_ref(),
        )
        .await
}

/// Returns a user's optional public presentation theme override.
///
/// An absent or unparseable stored value means the user inherits the site theme.
/// Database read failures propagate.
///
/// # Errors
///
/// Returns a database error when the configuration row cannot be read.
pub async fn get_theme_override(
    config: &dyn UserConfigStorage,
    user_id: UserId,
) -> Result<Option<Theme>> {
    let raw = UserConfigStorage::get(config, user_id, UserConfigKey::Theme).await?;
    Ok(raw.as_deref().and_then(|value| value.parse().ok()))
}

/// Stores a user's public presentation theme override.
///
/// # Errors
///
/// Returns a database error when the override cannot be stored.
pub async fn set_theme_override(
    config: &dyn UserConfigStorage,
    transaction: &mut WriteTransaction,
    user_id: UserId,
    theme: Theme,
) -> Result<()> {
    config
        .set(transaction, user_id, UserConfigKey::Theme, theme.as_ref())
        .await
}

/// Deletes a user's public presentation theme override, restoring site-theme inheritance.
///
/// # Errors
///
/// Returns a database error when the override cannot be deleted.
pub async fn delete_theme_override(
    config: &dyn UserConfigStorage,
    transaction: &mut WriteTransaction,
    user_id: UserId,
) -> Result<()> {
    config
        .delete(transaction, user_id, UserConfigKey::Theme)
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
    // plus the lossless stored-value row decode for `get` and the query-arguments bound.
    (StoredUserConfigValue,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    // `UserConfigKey`'s sqlx bridge reports `String` as its type (the token is bound as
    // borrowed text), so binding a key directly needs `String: Type<DB>` in scope.
    String: Type<DB>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> String: Encode<'q, DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.user_config.get",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get(&self, user_id: UserId, key: UserConfigKey) -> Result<Option<String>> {
        let row = sqlx::query_as::<_, (StoredUserConfigValue,)>(
            "SELECT value FROM user_config WHERE user_id = $1 AND key = $2",
        )
        .bind_storage(user_id)
        .bind_storage(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(value,)| value.into_inner()))
    }

    #[tracing::instrument(
        name = "storage.user_config.set",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn set(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        key: UserConfigKey,
        value: &str,
    ) -> Result<()> {
        set_stored::<DB>(
            transaction,
            user_id,
            key,
            StoredUserConfigValue(value.to_owned()),
        )
        .await
    }

    #[tracing::instrument(
        name = "storage.user_config.delete",
        skip(self, transaction),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn delete(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        key: UserConfigKey,
    ) -> Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("DELETE FROM user_config WHERE user_id = $1 AND key = $2")
            .bind_storage(user_id)
            .bind_storage(key)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }
}
async fn set_stored<DB>(
    transaction: &mut WriteTransaction,
    user_id: UserId,
    key: UserConfigKey,
    value: StoredUserConfigValue,
) -> Result<()>
where
    DB: Database + Backend,
    UserId: Type<DB>,
    for<'q> UserId: Encode<'q, DB>,
    UserConfigKey: Type<DB>,
    for<'q> UserConfigKey: Encode<'q, DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    StoredUserConfigValue: Type<DB>,
    for<'q> StoredUserConfigValue: Encode<'q, DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let connection = DB::write_connection(transaction)?;
    sqlx::query(
        "INSERT INTO user_config (user_id, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind_storage(user_id)
    .bind_storage(key)
    .bind_storage(value)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, SeedUser, backends};
    use common::MutationOutcome;
    use common::theme::Theme;
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn get_default_post_format_unset_returns_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let config = &*env.state.user_config;
        let result = get_default_post_format(config, user_id).await.unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_preserves_opaque_stored_values(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let config = std::sync::Arc::clone(&env.state.user_config);
        let config_for_write = std::sync::Arc::clone(&config);
        let key = UserConfigKey::DefaultPostFormat;
        let value = "unknown representation\nretained verbatim".to_owned();
        let expected = value.clone();
        let outcome = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    config_for_write
                        .set(transaction, user_id, key, &value)
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, MutationOutcome::Confirmed(())));

        assert_eq!(
            config
                .get(user_id, UserConfigKey::DefaultPostFormat)
                .await
                .unwrap(),
            Some(expected)
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_and_get_default_post_format_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let config = std::sync::Arc::clone(&env.state.user_config);
        let config_for_write = std::sync::Arc::clone(&config);
        let format = PostFormat::Markdown;
        let outcome = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    set_default_post_format(config_for_write.as_ref(), transaction, user_id, format)
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, MutationOutcome::Confirmed(())));
        let result = get_default_post_format(config.as_ref(), user_id)
            .await
            .unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn set_and_get_default_post_format_org(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let config = std::sync::Arc::clone(&env.state.user_config);
        let config_for_write = std::sync::Arc::clone(&config);
        let format = PostFormat::Org;
        let outcome = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    set_default_post_format(config_for_write.as_ref(), transaction, user_id, format)
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, MutationOutcome::Confirmed(())));
        let result = get_default_post_format(config.as_ref(), user_id)
            .await
            .unwrap();
        assert_eq!(result, PostFormat::Org);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_default_post_format_invalid_string_returns_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let config = std::sync::Arc::clone(&env.state.user_config);
        let config_for_write = std::sync::Arc::clone(&config);
        let key = UserConfigKey::DefaultPostFormat;
        let value = "garbage".to_owned();
        let outcome = env
            .state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    config_for_write
                        .set(transaction, user_id, key, &value)
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, MutationOutcome::Confirmed(())));

        let result = get_default_post_format(config.as_ref(), user_id)
            .await
            .unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }
    #[apply(backends)]
    #[tokio::test]
    async fn theme_override_is_none_when_absent(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        assert_eq!(
            get_theme_override(env.state.user_config.as_ref(), user_id)
                .await
                .unwrap(),
            None
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn theme_override_round_trips_and_delete_restores_inheritance(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let config = std::sync::Arc::clone(&env.state.user_config);
        for theme in [Theme::Terminal, Theme::Studio, Theme::Reader] {
            let config_for_write = std::sync::Arc::clone(&config);
            assert!(matches!(
                env.state
                    .write_scope
                    .run(move |transaction| {
                        Box::pin(async move {
                            set_theme_override(
                                config_for_write.as_ref(),
                                transaction,
                                user_id,
                                theme,
                            )
                            .await
                        })
                    })
                    .await
                    .unwrap(),
                MutationOutcome::Confirmed(())
            ));
            assert_eq!(
                get_theme_override(config.as_ref(), user_id).await.unwrap(),
                Some(theme)
            );
        }

        let config_for_delete = std::sync::Arc::clone(&config);
        assert!(matches!(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        delete_theme_override(config_for_delete.as_ref(), transaction, user_id)
                            .await
                    })
                })
                .await
                .unwrap(),
            MutationOutcome::Confirmed(())
        ));
        assert_eq!(
            get_theme_override(config.as_ref(), user_id).await.unwrap(),
            None
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn invalid_theme_override_is_none(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let config = std::sync::Arc::clone(&env.state.user_config);
        let config_for_write = std::sync::Arc::clone(&config);
        assert!(matches!(
            env.state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config_for_write
                            .set(transaction, user_id, UserConfigKey::Theme, "solarized")
                            .await
                    })
                })
                .await
                .unwrap(),
            MutationOutcome::Confirmed(())
        ));
        assert_eq!(
            get_theme_override(config.as_ref(), user_id).await.unwrap(),
            None
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn theme_override_propagates_database_errors(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        env.base.pool().close().await;
        assert!(matches!(
            get_theme_override(env.state.user_config.as_ref(), user_id).await,
            Err(sqlx::Error::PoolClosed)
        ));
    }
}

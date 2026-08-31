//! Per-user preference storage.

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::posts::PostFormat;
use crate::sql::QueryStorageExt;
use async_trait::async_trait;
use common::ids::UserId;
use sqlx::{Database, Pool};

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
    async fn get(&self, user_id: UserId, key: UserConfigKey) -> sqlx::Result<Option<String>>;

    /// Sets or updates a user's configuration value.
    async fn set(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        key: UserConfigKey,
        value: &str,
    ) -> sqlx::Result<()>;

    /// Deletes a specific configuration key for a user.
    async fn delete(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        key: UserConfigKey,
    ) -> sqlx::Result<()>;
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
) -> sqlx::Result<PostFormat> {
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
) -> sqlx::Result<()> {
    config
        .set(
            transaction,
            user_id,
            UserConfigKey::DefaultPostFormat,
            format.as_ref(),
        )
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
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `UserConfigKey`'s sqlx bridge reports `String` as its type (the token is bound as
    // borrowed text), so binding a key directly needs `String: Type<DB>` in scope.
    String: sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.user_config.get",
        skip(self),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn get(&self, user_id: UserId, key: UserConfigKey) -> sqlx::Result<Option<String>> {
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
    ) -> sqlx::Result<()> {
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
    ) -> sqlx::Result<()> {
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
) -> sqlx::Result<()>
where
    DB: Database + Backend,
    UserId: sqlx::Type<DB>,
    for<'q> UserId: sqlx::Encode<'q, DB>,
    UserConfigKey: sqlx::Type<DB>,
    for<'q> UserConfigKey: sqlx::Encode<'q, DB>,
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    StoredUserConfigValue: sqlx::Type<DB>,
    for<'q> StoredUserConfigValue: sqlx::Encode<'q, DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
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
}

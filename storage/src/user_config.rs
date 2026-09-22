//! Per-user preference storage.

use crate::WriteTransaction;
use crate::backend::Backend;
use crate::posts::models::PostFormat;
use crate::sql::QueryStorageExt;
use async_trait::async_trait;
use common::content_license::ContentLicense;
use common::ids::UserId;
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
#[cfg_attr(any(test, feature = "test-utils"), mockall::automock)]
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

    /// Returns a user's publication-wide Content License.
    ///
    /// A missing row preserves pre-setting databases and backups as All Rights
    /// Reserved. An explicit malformed value is rejected rather than defaulted,
    /// so corrupt stored rights data cannot silently widen publication rights.
    async fn get_content_license(&self, user_id: UserId) -> Result<ContentLicense> {
        match self.get(user_id, UserConfigKey::ContentLicense).await? {
            None => Ok(ContentLicense::default()),
            Some(value) => value
                .parse()
                .map_err(|error| sqlx::Error::Decode(Box::new(error))),
        }
    }

    /// Returns the effective Content License while participating in `transaction`.
    async fn get_content_license_for_update(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
    ) -> Result<ContentLicense>;

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

/// Backend-specific SQL fragments used by [`UserConfigStore`].
pub(crate) trait UserConfigDialect: Backend {
    /// Row-lock clause for state observed before mutation.
    const FOR_UPDATE: &'static str;
}

/// Generic [`UserConfigStorage`] backed by a [`UserConfigDialect`] database.
///
/// Shared SQL remains here; the backend modules own the row-lock divergence.
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
    DB: UserConfigDialect,
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
    DB::Arguments: sqlx::IntoArguments<DB>,
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

    async fn get_content_license_for_update(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
    ) -> Result<ContentLicense> {
        let connection = DB::write_connection(transaction)?;
        let sql = format!(
            "SELECT value FROM user_config WHERE user_id = $1 AND key = $2{}",
            DB::FOR_UPDATE
        );
        let row = sqlx::query_as::<_, (StoredUserConfigValue,)>(sqlx::AssertSqlSafe(sql))
            .bind_storage(user_id)
            .bind_storage(UserConfigKey::ContentLicense)
            .fetch_optional(&mut *connection)
            .await?;
        match row {
            None => Ok(ContentLicense::default()),
            Some((value,)) => value
                .into_inner()
                .parse()
                .map_err(|error| sqlx::Error::Decode(Box::new(error))),
        }
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
    DB::Arguments: sqlx::IntoArguments<DB>,
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
    use strum::VariantArray as _;

    #[apply(backends)]
    #[tokio::test]
    async fn get_default_post_format_unset_returns_markdown(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let config = &*env.user_config();
        let result = get_default_post_format(config, user_id).await.unwrap();
        assert_eq!(result, PostFormat::Markdown);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn content_license_missing_row_defaults_to_all_rights_reserved(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;

        assert_eq!(
            env.user_config()
                .get_content_license(user_id)
                .await
                .unwrap(),
            ContentLicense::AllRightsReserved
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn content_license_round_trips_every_choice(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        for &license in ContentLicense::VARIANTS {
            let config = std::sync::Arc::clone(&env.user_config());
            let config_for_write = std::sync::Arc::clone(&config);
            let outcome = env
                .write_scope()
                .run(move |transaction| {
                    Box::pin(async move {
                        config_for_write
                            .set(
                                transaction,
                                user_id,
                                UserConfigKey::ContentLicense,
                                license.as_ref(),
                            )
                            .await
                    })
                })
                .await
                .unwrap();
            assert!(matches!(outcome, MutationOutcome::Confirmed(())));
            assert_eq!(config.get_content_license(user_id).await.unwrap(), license);
        }
    }

    #[apply(backends)]
    #[tokio::test]
    async fn content_license_invalid_explicit_value_fails_closed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let config = std::sync::Arc::clone(&env.user_config());
        let config_for_write = std::sync::Arc::clone(&config);
        let outcome = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    config_for_write
                        .set(transaction, user_id, UserConfigKey::ContentLicense, "MIT")
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, MutationOutcome::Confirmed(())));
        assert!(config.get_content_license(user_id).await.is_err());

        let config_for_read = std::sync::Arc::clone(&config);
        let outcome = env
            .write_scope()
            .run(move |transaction| {
                Box::pin(async move {
                    assert!(
                        config_for_read
                            .get_content_license_for_update(transaction, user_id)
                            .await
                            .is_err()
                    );
                    Ok::<(), sqlx::Error>(())
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, MutationOutcome::Confirmed(())));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_preserves_opaque_stored_values(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let config = std::sync::Arc::clone(&env.user_config());
        let config_for_write = std::sync::Arc::clone(&config);
        let key = UserConfigKey::DefaultPostFormat;
        let value = "unknown representation\nretained verbatim".to_owned();
        let expected = value.clone();
        let outcome = env
            .write_scope()
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
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let config = std::sync::Arc::clone(&env.user_config());
        let config_for_write = std::sync::Arc::clone(&config);
        let format = PostFormat::Markdown;
        let outcome = env
            .write_scope()
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
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let config = std::sync::Arc::clone(&env.user_config());
        let config_for_write = std::sync::Arc::clone(&config);
        let format = PostFormat::Org;
        let outcome = env
            .write_scope()
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
        let user_id = SeedUser::new()
            .seed(
                std::sync::Arc::clone(&env.users()),
                env.write_scope().clone(),
            )
            .await
            .user_id;
        let config = std::sync::Arc::clone(&env.user_config());
        let config_for_write = std::sync::Arc::clone(&config);
        let key = UserConfigKey::DefaultPostFormat;
        let value = "garbage".to_owned();
        let outcome = env
            .write_scope()
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

use std::env::VarError;
use std::io;
use std::sync::Arc;

use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use thiserror::Error;

use super::{
    PostgresAtomicOps, PostgresAudienceStorage, PostgresEmailVerificationStorage,
    PostgresFeedCacheStorage, PostgresFeedEventStorage, PostgresInviteStorage,
    PostgresMediaStorage, PostgresPasswordResetStorage, PostgresPostStorage,
    PostgresSessionStorage, PostgresSiteConfigStorage, PostgresSubscriptionStorage,
    PostgresUserConfigStorage, PostgresUserStorage,
};
use crate::instance_identity::ensure_instance_identity;
use crate::posts::backfill_post_media_references;

fn make_postgres_app_state(pool: PgPool) -> Arc<crate::AppState> {
    Arc::new(crate::AppState {
        site_config: Arc::new(PostgresSiteConfigStorage::new(pool.clone())),
        users: Arc::new(PostgresUserStorage::new(pool.clone())),
        sessions: Arc::new(PostgresSessionStorage::new(pool.clone())),
        invites: Arc::new(PostgresInviteStorage::new(pool.clone())),
        atomic: Arc::new(PostgresAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(PostgresEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(PostgresPasswordResetStorage::new(pool.clone())),
        posts: Arc::new(PostgresPostStorage::new(pool.clone())),
        subscriptions: Arc::new(PostgresSubscriptionStorage::new(
            pool.clone(),
            Arc::new(common::visibility::OpenSubscriptionPolicy),
        )),
        audiences: Arc::new(PostgresAudienceStorage::new(pool.clone())),
        media: Arc::new(PostgresMediaStorage::new(pool.clone())),
        user_config: Arc::new(PostgresUserConfigStorage::new(pool.clone())),
        feed_cache: Arc::new(PostgresFeedCacheStorage::new(pool.clone())),
        feed_events: Arc::new(PostgresFeedEventStorage::new(pool)),
    })
}

#[derive(Debug, Error)]
enum PostgresPasswordError {
    #[error("JAUNDER_DB_PASSWORD_FILE is not valid Unicode")]
    FileVariable(#[source] VarError),
    #[error("JAUNDER_DB_PASSWORD is not valid Unicode")]
    Variable(#[source] VarError),
    #[error("configured PostgreSQL password file could not be read")]
    FileRead(#[source] io::Error),
}

fn postgres_password_from_ops(
    mut read_variable: impl FnMut(&str) -> Result<String, VarError>,
    mut read_file: impl FnMut(&str) -> io::Result<String>,
) -> Result<Option<String>, PostgresPasswordError> {
    match read_variable("JAUNDER_DB_PASSWORD_FILE") {
        Ok(path) => {
            let password = read_file(&path).map_err(PostgresPasswordError::FileRead)?;
            return Ok(Some(password.trim_end().to_owned()));
        }
        Err(VarError::NotPresent) => {}
        Err(error @ VarError::NotUnicode(_)) => {
            return Err(PostgresPasswordError::FileVariable(error));
        }
    }

    match read_variable("JAUNDER_DB_PASSWORD") {
        Ok(password) => Ok(Some(password)),
        Err(VarError::NotPresent) => Ok(None),
        Err(error @ VarError::NotUnicode(_)) => Err(PostgresPasswordError::Variable(error)),
    }
}

fn read_password_variable(name: &str) -> Result<String, VarError> {
    std::env::var(name)
}

fn read_password_file(path: &str) -> io::Result<String> {
    std::fs::read_to_string(path)
}

fn postgres_password_from_env() -> Result<Option<String>, PostgresPasswordError> {
    postgres_password_from_ops(read_password_variable, read_password_file)
}

fn apply_postgres_password(
    options: &PgConnectOptions,
    password: Result<Option<String>, PostgresPasswordError>,
) -> sqlx::Result<PgConnectOptions> {
    let mut options = options.clone();
    if let Some(password) = password.map_err(|error| sqlx::Error::Configuration(Box::new(error)))? {
        options = options.password(&password);
    }
    Ok(options)
}

/// Resolve final Postgres options, applying password overrides from env
/// and the slow-query log threshold.
///
/// # Errors
///
/// Returns a configuration error retaining the typed environment or file source
/// when a configured password input cannot be read.
pub fn resolved_postgres_options(options: &PgConnectOptions) -> sqlx::Result<PgConnectOptions> {
    let options = apply_postgres_password(options, postgres_password_from_env())?;
    Ok(options.log_slow_statements(LevelFilter::Warn, crate::db::sql_slow_query_threshold()))
}

#[tracing::instrument(name = "storage.postgres.open_database", skip(options))]
pub(crate) async fn open_postgres_database_with_pool(
    options: &PgConnectOptions,
) -> sqlx::Result<(Arc<crate::AppState>, PgPool, crate::InstanceId)> {
    let options = resolved_postgres_options(options)?;
    let pool = PgPool::connect_with(options).await?;
    sqlx::migrate!("./migrations/postgres").run(&pool).await?;
    let instance_id = ensure_instance_identity(&pool).await?;
    backfill_post_media_references(&pool).await?;
    Ok((make_postgres_app_state(pool.clone()), pool, instance_id))
}

/// Returns `true` if the `PostgreSQL` database holds no user data — every table
/// except the migration-seeded lookups is empty.
pub(crate) async fn database_is_empty(options: &PgConnectOptions) -> sqlx::Result<bool> {
    let options = resolved_postgres_options(options)?;
    let pool = PgPool::connect_with(options).await?;
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
           AND table_name <> '_sqlx_migrations'",
    )
    .fetch_all(&pool)
    .await?;
    for table in tables {
        if crate::db::MIGRATION_SEEDED_TABLES.contains(&table.as_str()) {
            continue;
        }
        let has_row = sqlx::query_scalar::<_, bool>(&format!(
            "SELECT EXISTS(SELECT 1 FROM {} LIMIT 1)",
            crate::sql::quote_identifier(&table)
        ))
        .fetch_one(&pool)
        .await?;
        if has_row {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, CloseablePool, postgres_only};
    use common::tag::Tag;
    use common::test_support::{parse_tag, with_env};
    use rstest::*;
    use rstest_reuse::*;

    // reason: Postgres-only by nature, not by omission — `SQLite` has no array type,
    // so there is no parity behaviour to match. Per ADR-0053 a dialect feature with no
    // generic home lives with its dialect code. #891: a slice of a `StrNewtype` binds
    // as a `TEXT[]`, so a typed call site needs no strip. `feed_events` proves the
    // `i64` case (its `= ANY($n)` binds are the only ones in `storage/`); this is the
    // `String` one, so both halves of the bridge's array support are covered.
    #[apply(postgres_only)]
    #[tokio::test]
    async fn str_newtype_slices_bind_as_a_postgres_array(#[case] backend: Backend) {
        let env = backend.setup().await;
        let CloseablePool::Postgres(pool) = env.base.pool() else {
            unreachable!("postgres_only yields a Postgres pool")
        };

        for slug in ["alpha", "beta", "gamma"] {
            sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1)")
                .bind(slug)
                .execute(pool)
                .await
                .expect("seed tag");
        }

        // The point of the test: `&[Tag]` binds directly, with no `Vec<String>` strip.
        let wanted = vec![parse_tag("alpha"), parse_tag("gamma")];
        let found = sqlx::query_scalar::<_, Tag>(
            "SELECT tag_slug FROM tags WHERE tag_slug = ANY($1) ORDER BY tag_slug",
        )
        .bind(&wanted)
        .fetch_all(pool)
        .await
        .expect("array bind");

        assert_eq!(found, wanted);
    }

    fn assert_configuration_source<T: std::error::Error + 'static>(
        result: sqlx::Result<PgConnectOptions>,
        forbidden: &str,
    ) {
        let error = result.expect_err("option resolution must fail");
        assert!(
            matches!(&error, sqlx::Error::Configuration(_)),
            "credential input failures are configuration failures"
        );
        assert!(
            !error.to_string().contains(forbidden),
            "configuration context must not render credential bytes"
        );

        let mut source: &(dyn std::error::Error + 'static) = &error;
        let mut found = false;
        while let Some(next) = source.source() {
            if next.downcast_ref::<T>().is_some() {
                found = true;
                break;
            }
            source = next;
        }
        assert!(found, "typed source must remain downcastable");
    }

    #[test]
    fn postgres_password_prefers_file_over_env() {
        with_env(|env| {
            env.set("JAUNDER_DB_PASSWORD", "from-env");
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("db-password");
            std::fs::write(&path, "from-file\n").unwrap();
            env.set("JAUNDER_DB_PASSWORD_FILE", &path);

            let password = postgres_password_from_env().unwrap();

            assert_eq!(password.as_deref(), Some("from-file"));
        });
    }

    #[test]
    fn postgres_password_uses_env_when_file_unset() {
        with_env(|env| {
            env.remove("JAUNDER_DB_PASSWORD_FILE");
            env.set("JAUNDER_DB_PASSWORD", "from-env");

            let password = postgres_password_from_env().unwrap();

            assert_eq!(password.as_deref(), Some("from-env"));
        });
    }

    #[test]
    fn postgres_password_returns_none_when_unset() {
        with_env(|env| {
            env.remove("JAUNDER_DB_PASSWORD");
            env.remove("JAUNDER_DB_PASSWORD_FILE");

            let password = postgres_password_from_env().unwrap();

            assert_eq!(password, None);
        });
    }

    #[test]
    fn resolved_postgres_options_applies_password_override_when_env_set() {
        with_env(|env| {
            env.set("JAUNDER_DB_PASSWORD", "secret");
            env.remove("JAUNDER_DB_PASSWORD_FILE");

            let base: PgConnectOptions = "postgres://user@localhost/db".parse().unwrap();
            let resolved = resolved_postgres_options(&base);

            assert!(resolved.is_ok());
        });
    }

    #[test]
    fn postgres_password_from_env_invalid_file_variable_fails_closed() {
        let marker = "file-credential-byte-marker";
        let password = postgres_password_from_ops(
            |key| {
                assert_eq!(key, "JAUNDER_DB_PASSWORD_FILE");
                Err(VarError::NotUnicode(std::ffi::OsString::from(marker)))
            },
            |_| unreachable!("an invalid file variable cannot name a file"),
        );
        let base: PgConnectOptions = "postgres://user@localhost/db".parse().unwrap();

        assert_configuration_source::<VarError>(apply_postgres_password(&base, password), marker);
    }

    #[test]
    fn postgres_password_from_env_invalid_direct_variable_fails_closed() {
        let marker = "direct-credential-byte-marker";
        let password = postgres_password_from_ops(
            |key| match key {
                "JAUNDER_DB_PASSWORD_FILE" => Err(VarError::NotPresent),
                "JAUNDER_DB_PASSWORD" => {
                    Err(VarError::NotUnicode(std::ffi::OsString::from(marker)))
                }
                _ => unreachable!("only the two credential variables are read"),
            },
            |_| unreachable!("an absent file variable cannot cause a file read"),
        );
        let base: PgConnectOptions = "postgres://user@localhost/db".parse().unwrap();

        assert_configuration_source::<VarError>(apply_postgres_password(&base, password), marker);
    }

    #[test]
    fn resolved_postgres_options_retains_io_source_when_password_file_unreadable() {
        with_env(|env| {
            env.remove("JAUNDER_DB_PASSWORD");
            let missing_path = "/nonexistent/path/to/db-password";
            env.set("JAUNDER_DB_PASSWORD_FILE", missing_path);

            let base: PgConnectOptions = "postgres://user@localhost/db".parse().unwrap();
            let result = resolved_postgres_options(&base);

            assert_configuration_source::<io::Error>(result, missing_path);
        });
    }
}

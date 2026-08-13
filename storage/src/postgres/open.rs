use std::io;
use std::sync::Arc;

use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;

use super::{
    PostgresAtomicOps, PostgresAudienceStorage, PostgresEmailVerificationStorage,
    PostgresFeedCacheStorage, PostgresFeedEventStorage, PostgresInviteStorage,
    PostgresMediaStorage, PostgresPasswordResetStorage, PostgresPostStorage,
    PostgresSessionStorage, PostgresSiteConfigStorage, PostgresSubscriptionStorage,
    PostgresUserConfigStorage, PostgresUserStorage,
};

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

fn postgres_password_from_env() -> io::Result<Option<String>> {
    if let Ok(path) = std::env::var("JAUNDER_DB_PASSWORD_FILE") {
        return std::fs::read_to_string(path).map(|s| Some(s.trim_end().to_owned()));
    }

    Ok(std::env::var("JAUNDER_DB_PASSWORD").ok())
}

/// Resolve final Postgres options, applying password overrides from env
/// and the slow-query log threshold.
///
/// # Errors
///
/// Returns `sqlx::Error::Io` if the password file env var points at an
/// unreadable file.
pub fn resolved_postgres_options(options: &PgConnectOptions) -> sqlx::Result<PgConnectOptions> {
    let mut options = options.clone();
    if let Some(password) = postgres_password_from_env().map_err(sqlx::Error::Io)? {
        options = options.password(&password);
    }
    options = options.log_slow_statements(LevelFilter::Warn, crate::db::sql_slow_query_threshold());
    Ok(options)
}

#[tracing::instrument(name = "storage.postgres.open_database", skip(options))]
pub(crate) async fn open_postgres_database_with_pool(
    options: &PgConnectOptions,
) -> sqlx::Result<(Arc<crate::AppState>, PgPool)> {
    let options = resolved_postgres_options(options)?;
    let pool = PgPool::connect_with(options).await?;
    sqlx::migrate!("./migrations/postgres").run(&pool).await?;
    Ok((make_postgres_app_state(pool.clone()), pool))
}

/// Opens the `PostgreSQL` database and returns just the [`AppState`]; the pool is
/// dropped. Tests that need to inject a pool fault use
/// [`open_postgres_database_with_pool`] via the `test_support` harness.
pub(crate) async fn open_postgres_database(
    options: &PgConnectOptions,
) -> sqlx::Result<Arc<crate::AppState>> {
    Ok(open_postgres_database_with_pool(options).await?.0)
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
    fn resolved_postgres_options_returns_io_error_when_password_file_unreadable() {
        with_env(|env| {
            env.remove("JAUNDER_DB_PASSWORD");
            env.set(
                "JAUNDER_DB_PASSWORD_FILE",
                "/nonexistent/path/to/db-password",
            );

            let base: PgConnectOptions = "postgres://user@localhost/db".parse().unwrap();
            let result = resolved_postgres_options(&base);

            assert!(matches!(result, Err(sqlx::Error::Io(_))));
        });
    }
}

use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use std::sync::Arc;

use crate::backup::CatalogTableName;
use crate::posts::store;
use crate::sql::Exists;
use crate::{instance_identity, make_app_state};

/// Resolve final Postgres options from the application connection snapshot.
#[must_use]
pub fn resolved_postgres_options(
    options: &PgConnectOptions,
    runtime: &crate::StorageRuntimeConfig,
) -> PgConnectOptions {
    let mut options = options.clone();
    if let Some(password) = runtime.postgres_password() {
        options = options.password(&password.0);
    }
    options.log_slow_statements(LevelFilter::Warn, runtime.sql_slow_query_threshold())
}

#[tracing::instrument(name = "storage.postgres.open_database", skip(options, runtime))]
pub(crate) async fn open_postgres_database_with_pool(
    options: &PgConnectOptions,
    runtime: &crate::StorageRuntimeConfig,
) -> sqlx::Result<(Arc<crate::AppState>, PgPool, crate::InstanceId)> {
    let options = resolved_postgres_options(options, runtime);
    let pool = PgPool::connect_with(options).await?;
    sqlx::migrate!("./migrations/postgres").run(&pool).await?;
    let instance_id = instance_identity::ensure(&pool).await?;
    store::backfill_post_media_references(&pool).await?;
    Ok((make_app_state(pool.clone()), pool, instance_id))
}

/// Returns `true` if the `PostgreSQL` database holds no user data — every table
/// except the migration-seeded lookups is empty.
pub(crate) async fn database_is_empty(
    options: &PgConnectOptions,
    runtime: &crate::StorageRuntimeConfig,
) -> sqlx::Result<bool> {
    let options = resolved_postgres_options(options, runtime);
    let pool = PgPool::connect_with(options).await?;
    let tables = sqlx::query_scalar::<_, CatalogTableName>(
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
        let has_row = sqlx::query_scalar::<_, Exists>(&format!(
            "SELECT EXISTS(SELECT 1 FROM {} LIMIT 1)",
            crate::sql::quote_identifier(table.as_str())
        ))
        .fetch_one(&pool)
        .await?
        .into_bool();
        if has_row {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::QueryStorageExt;
    use crate::test_support::{Backend, CloseablePool, postgres_only};
    use common::tag::Tag;
    use common::test_support::parse_tag;
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
            let tag = parse_tag(slug);
            sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1)")
                .bind_storage(tag)
                .execute(pool)
                .await
                .expect("seed tag");
        }

        // The point of the test: `&[Tag]` binds directly, with no `Vec<String>` strip.
        let wanted = vec![parse_tag("alpha"), parse_tag("gamma")];
        let found = sqlx::query_scalar::<_, Tag>(
            "SELECT tag_slug FROM tags WHERE tag_slug = ANY($1) ORDER BY tag_slug",
        )
        .bind_storage(&wanted)
        .fetch_all(pool)
        .await
        .expect("array bind");

        assert_eq!(found, wanted);
    }

    fn raw_config(
        password_file: Result<Option<std::io::Result<String>>, std::env::VarError>,
        password: Result<Option<String>, std::env::VarError>,
    ) -> Result<crate::StorageRuntimeConfig, crate::PostgresPasswordError> {
        crate::StorageRuntimeConfig::from_raw(Ok(None), password_file, password)
    }

    #[test]
    fn postgres_password_prefers_trimmed_file_value_over_variable() {
        let runtime = raw_config(
            Ok(Some(Ok("from-file\n".to_owned()))),
            Ok(Some("from-variable".to_owned())),
        )
        .expect("file password resolves");

        assert_eq!(
            runtime
                .postgres_password()
                .map(|password| password.0.as_str()),
            Some("from-file")
        );
    }

    #[test]
    fn postgres_password_allows_empty_file_override() {
        let runtime = raw_config(
            Ok(Some(Ok("\n".to_owned()))),
            Ok(Some("fallback".to_owned())),
        )
        .expect("empty file password is a valid override");

        assert_eq!(
            runtime
                .postgres_password()
                .map(|password| password.0.as_str()),
            Some("")
        );
    }

    #[test]
    fn postgres_options_apply_the_runtime_password_override() {
        let runtime = raw_config(Ok(None), Ok(Some("override".to_owned())))
            .expect("password override resolves");
        let base: PgConnectOptions = "postgres://user:embedded@localhost/db".parse().unwrap();

        let resolved = resolved_postgres_options(&base, &runtime);

        assert_ne!(format!("{resolved:?}"), format!("{base:?}"));
    }

    #[test]
    fn postgres_password_falls_back_to_embedded_url_when_unset() {
        let runtime = raw_config(Ok(None), Ok(None)).expect("no override resolves");
        let base: PgConnectOptions = "postgres://user:embedded@localhost/db".parse().unwrap();

        let resolved = resolved_postgres_options(&base, &runtime);
        assert_ne!(format!("{resolved:?}"), format!("{base:?}"));
    }

    #[test]
    fn postgres_password_invalid_variable_retains_typed_source_without_secret() {
        let marker = "credential-byte-marker";
        let Err(error) = raw_config(
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                marker,
            ))),
            Ok(None),
        ) else {
            unreachable!("invalid password-file variable must fail closed");
        };

        assert!(matches!(
            error,
            crate::PostgresPasswordError::FileVariable(_)
        ));
        assert!(!error.to_string().contains(marker));
    }

    #[test]
    fn postgres_password_unreadable_file_retains_typed_source_without_path() {
        let path = "/private/password-file";
        let Err(error) = raw_config(
            Ok(Some(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                path,
            )))),
            Ok(None),
        ) else {
            unreachable!("unreadable password file must fail closed");
        };

        assert!(matches!(error, crate::PostgresPasswordError::FileRead(_)));
        assert!(!error.to_string().contains(path));
    }
}

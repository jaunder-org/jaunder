use std::sync::Arc;

use host::config_key::SiteConfigKey;

use crate::cli::StorageArgs;

use super::support;

/// Upsert a `site_config` key/value through the real storage path.
///
/// The value is checked against the key's own validator *before* the database is
/// opened, so a rejected value never reaches a row.
pub(super) async fn cmd_site_config_set(
    storage: &StorageArgs,
    key: SiteConfigKey,
    value: &str,
) -> anyhow::Result<()> {
    key.validate(value)?;
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    let site_config = Arc::clone(&state.site_config);
    let value = value.to_owned();
    let value_for_set = value.clone();
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { site_config.set(transaction, key, &value_for_set).await })
        })
        .await?;
    support::require_confirmed_mutation(outcome, "site_config set")?;
    eprintln!("set site_config {key} = {value}");
    Ok(())
}

/// Print the value for `key` to stdout; error (→ non-zero exit) if it is unset,
/// so a caller can distinguish an unset key from an empty value.
pub(super) async fn cmd_site_config_get(
    storage: &StorageArgs,
    key: SiteConfigKey,
) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    match state.site_config.get_raw(key).await? {
        Some(value) => {
            println!("{value}");
            Ok(())
        }
        None => Err(anyhow::anyhow!("no site_config value for key {key:?}")),
    }
}

/// Print all `site_config` entries as `key=value`, one per line, ordered by key.
pub(super) async fn cmd_site_config_list(storage: &StorageArgs) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    let entries = state.site_config.list().await?;
    print!("{}", format_entries(&entries));
    Ok(())
}

/// Delete a `site_config` key. Idempotent (exit 0 whether or not a row existed);
/// stderr notes which happened.
pub(super) async fn cmd_site_config_unset(
    storage: &StorageArgs,
    key: SiteConfigKey,
) -> anyhow::Result<()> {
    let runtime = support::storage_runtime_config(&storage.db)?;
    let state = storage::open_existing_database(&storage.db, &runtime).await?;
    let site_config = Arc::clone(&state.site_config);
    let outcome = state
        .write_scope
        .run(move |transaction| Box::pin(async move { site_config.delete(transaction, key).await }))
        .await?;
    let removed = support::require_confirmed_mutation(outcome, "site_config unset")?;
    if removed {
        eprintln!("unset site_config {key}");
    } else {
        eprintln!("site_config {key} was not set (no-op)");
    }
    Ok(())
}

/// Render `site_config` entries as `key=value\n` lines (a human/discovery view;
/// `site-config get` is the lossless scriptable accessor). Pure, unit-tested directly.
///
/// Every stored row is printed — this is a faithful dump of what is physically
/// stored — but rows the registry judges are annotated (spec D4):
///
/// - a key outside [`SiteConfigKey`] is marked `UNKNOWN KEY` (a legacy or
///   hand-written row the typed seam can no longer read or write);
/// - a known key whose value fails its validator is marked `INVALID (<reason>)`.
///
/// An empty value on an optional key is *not* invalid: empty means unset (spec
/// D1b), which `SiteConfigKey::validate` already honours.
fn format_entries(entries: &[(String, String)]) -> String {
    use std::fmt::Write;

    entries.iter().fold(String::new(), |mut out, (k, v)| {
        // writeln! to a String is infallible; the newline gives one entry per line.
        let _ = match k.parse::<SiteConfigKey>() {
            Err(_) => writeln!(out, "{:<40}  UNKNOWN KEY", format!("{k}={v}")),
            Ok(key) => match key.validate(v) {
                Ok(()) => writeln!(out, "{k}={v}"),
                Err(err) => writeln!(out, "{:<40}  INVALID ({err})", format!("{k}={v}")),
            },
        };
        out
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use rstest::*;
    use rstest_reuse::*;
    use storage::{
        StorageRuntimeConfig,
        test_support::{
            Backend, PostgresDbGuard, TestEnv, backends, confirmed, sqlite_url, unique_postgres_url,
        },
    };
    use tempfile::TempDir;

    use super::super::test_support::sqlite_storage_args;

    /// A `StorageArgs` for `backend` whose database already exists, since the
    /// `site-config` handlers all go through `open_existing_database`.
    async fn site_config_args(
        backend: Backend,
        base: &TempDir,
    ) -> (StorageArgs, Option<PostgresDbGuard>) {
        let (db, guard) = match backend {
            Backend::Sqlite => (sqlite_url(base), None),
            Backend::Postgres => {
                let config = storage::test_support::PostgresTestConfig::from_env();
                let (db, guard) = unique_postgres_url(&config).await;
                (db, Some(guard))
            }
        };
        storage::open_database(&db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");
        (
            StorageArgs {
                storage_path: base.path().to_path_buf(),
                db,
            },
            guard,
        )
    }

    #[test]
    fn format_entries_renders_sorted_key_value_lines() {
        let entries = vec![
            (
                "site.base_url".to_string(),
                "https://example.com/".to_string(),
            ),
            ("site.title".to_string(), "My Site".to_string()),
        ];
        assert_eq!(
            format_entries(&entries),
            "site.base_url=https://example.com/\nsite.title=My Site\n"
        );
        assert_eq!(format_entries(&[]), "");
    }

    /// A7: a known key with an invalid value is rejected before the write.
    #[apply(backends)]
    #[tokio::test]
    async fn site_config_set_rejects_an_invalid_value(#[case] backend: Backend) {
        let base = TempDir::new().expect("temp dir");
        let (args, _pg) = site_config_args(backend, &base).await;
        let state = storage::open_existing_database(&args.db, &StorageRuntimeConfig::default())
            .await
            .expect("reopen");
        let before = state.site_config.list().await.unwrap().len();

        cmd_site_config_set(&args, SiteConfigKey::SiteBaseUrl, "nonsense://x")
            .await
            .expect_err("an unparseable base URL is refused");

        assert_eq!(
            state.site_config.list().await.unwrap().len(),
            before,
            "no row written",
        );
    }

    /// A8: empty-means-unset survives at the CLI door.
    #[apply(backends)]
    #[tokio::test]
    async fn site_config_set_accepts_empty_for_an_optional_key(#[case] backend: Backend) {
        let base = TempDir::new().expect("temp dir");
        let (args, _pg) = site_config_args(backend, &base).await;

        cmd_site_config_set(&args, SiteConfigKey::SiteBaseUrl, "")
            .await
            .expect("empty means unset on an optional key");

        let state = storage::open_existing_database(&args.db, &StorageRuntimeConfig::default())
            .await
            .expect("reopen");
        assert_eq!(
            state
                .site_config
                .get_raw(SiteConfigKey::SiteBaseUrl)
                .await
                .unwrap(),
            Some(String::new()),
        );
    }

    /// A9: list is a faithful dump that judges without hiding.
    #[apply(backends)]
    #[tokio::test]
    async fn site_config_list_flags_unknown_keys_and_invalid_values(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        // A row the registry does not know. `set` cannot express it any more, which is
        // exactly the legacy case `list` exists to surface -- so write it as raw SQL
        // through the harness pool.
        base.pool()
            .execute("INSERT INTO site_config (key, value) VALUES ('legacy.orphan', 'x')")
            .await
            .unwrap();
        let cfg = &state.site_config;
        // set() is the typed seam and does not validate; the CLI does. Storing junk
        // here is how a pre-#687 row would look to `list`.
        let config = Arc::clone(&state.site_config);
        confirmed(
            state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config
                            .set(transaction, SiteConfigKey::SiteBaseUrl, "nonsense://x")
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        let config = Arc::clone(&state.site_config);
        confirmed(
            state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config
                            .set(transaction, SiteConfigKey::SiteTitle, "My Site")
                            .await
                    })
                })
                .await
                .unwrap(),
        );
        // An empty value on an optional key means unset, not invalid (spec D1b).
        let config = Arc::clone(&state.site_config);
        confirmed(
            state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        config
                            .set(transaction, SiteConfigKey::BackupDestinationPath, "")
                            .await
                    })
                })
                .await
                .unwrap(),
        );

        let rendered = format_entries(&cfg.list().await.unwrap());

        let line = |prefix: &str| {
            rendered
                .lines()
                .find(|l| l.starts_with(prefix))
                .unwrap_or_else(|| panic!("no line for {prefix} in:\n{rendered}"))
                .to_owned()
        };
        assert!(line("legacy.orphan=x").contains("UNKNOWN KEY"));
        assert!(line("site.base_url=nonsense://x").contains("INVALID"));
        assert_eq!(line("site.title="), "site.title=My Site");
        assert_eq!(
            line("backup.destination_path="),
            "backup.destination_path=",
            "empty on an optional key is unset, not invalid",
        );
    }

    #[tokio::test]
    async fn cmd_site_config_set_upserts_and_get_and_list_read_back() {
        let temp = TempDir::new().expect("temp dir");
        let storage_args = sqlite_storage_args(&temp);
        // Handlers use open_existing_database, so the DB must already exist.
        storage::open_database(&storage_args.db, &StorageRuntimeConfig::default())
            .await
            .expect("open db");

        cmd_site_config_set(
            &storage_args,
            SiteConfigKey::FeedsWebsubHubUrl,
            "https://x/",
        )
        .await
        .expect("set ok");
        // set() is an upsert: a second write on the same key overwrites.
        cmd_site_config_set(
            &storage_args,
            SiteConfigKey::FeedsWebsubHubUrl,
            "https://y/",
        )
        .await
        .expect("upsert ok");

        let state =
            storage::open_existing_database(&storage_args.db, &StorageRuntimeConfig::default())
                .await
                .expect("reopen");
        assert_eq!(
            state
                .site_config
                .get_raw(SiteConfigKey::FeedsWebsubHubUrl)
                .await
                .unwrap(),
            Some("https://y/".to_string()),
            "second set overwrites",
        );

        // get: present key returns Ok (exercises the println! path); an unwritten key
        // errors. A key outside the registry can no longer be named here at all — clap
        // rejects it at parse time (see `cli`'s `site_config_rejects_an_unknown_key`).
        cmd_site_config_get(&storage_args, SiteConfigKey::FeedsWebsubHubUrl)
            .await
            .expect("get present key ok");
        cmd_site_config_get(&storage_args, SiteConfigKey::SiteTitle)
            .await
            .expect_err("get unwritten key errors (→ non-zero exit)");

        // list runs against a populated store (exercises the print path).
        cmd_site_config_list(&storage_args).await.expect("list ok");
    }
}

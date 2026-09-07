//! Public backup operations and their database, archive, and media sequencing.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

use sha2::{Digest, Sha256};

use crate::{DbConnectOptions, StorageRuntimeConfig, postgres, sqlite};

use super::{
    BackupMode, archive,
    error::BackupError,
    format::{self, BackupManifest},
    media,
    restore_validation::{BackupRestoreOutcome, RestoreValidationReport},
};

#[derive(Clone, Copy)]
pub struct BackupExportOptions<'a> {
    pub database: &'a DbConnectOptions,
    pub runtime: &'a StorageRuntimeConfig,
    pub media_path: &'a Path,
    pub destination_path: &'a Path,
    pub mode: BackupMode,
}

#[derive(Clone, Copy)]
pub struct BackupRestoreOptions<'a> {
    pub database: &'a DbConnectOptions,
    pub runtime: &'a StorageRuntimeConfig,
    pub media_path: &'a Path,
    pub source_path: &'a Path,
}

/// # Errors
///
/// Returns `Err(BackupError)` if the backup export fails.
pub async fn export_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    match options.mode {
        BackupMode::Directory => export_directory_backup(options).await,
        BackupMode::Archive => export_archive_backup(options).await,
    }
}

/// # Errors
///
/// Returns `Err(BackupError)` if the backup restore fails.
pub async fn restore_backup(
    options: BackupRestoreOptions<'_>,
) -> Result<BackupRestoreOutcome, BackupError> {
    let extracted_archive = if options.source_path.is_file() {
        Some(archive::extract_archive_backup(options.source_path)?)
    } else {
        None
    };
    let source_path = extracted_archive
        .as_ref()
        .map_or(options.source_path, archive::TemporaryBackupDirectory::path);

    let manifest = format::read_manifest(source_path)?;
    format::validate_manifest(&manifest)?;

    validate_theme_content_backup(source_path)?;
    preflight_directory_backup(
        BackupRestoreOptions {
            database: options.database,
            runtime: options.runtime,
            media_path: options.media_path,
            source_path,
        },
        &manifest,
    )
    .await?;
    let content_root = options.media_path.parent().ok_or_else(|| {
        BackupError::InvalidBackup("media storage path has no content-root parent".into())
    })?;
    media::restore_media_directory(&source_path.join("themes"), &content_root.join("themes"))?;
    let validation_report = match manifest.mode {
        BackupMode::Directory | BackupMode::Archive => {
            restore_directory_backup(
                BackupRestoreOptions {
                    database: options.database,
                    runtime: options.runtime,
                    media_path: options.media_path,
                    source_path,
                },
                &manifest,
            )
            .await?
        }
    };
    media::restore_media_directory(&source_path.join("media"), options.media_path)?;

    Ok(BackupRestoreOutcome {
        manifest,
        validation_report,
    })
}

async fn export_archive_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    archive::ensure_absent(options.destination_path)?;
    let staging = archive::TemporaryBackupDirectory::near(options.destination_path)?;
    let manifest = export_directory_backup(BackupExportOptions {
        database: options.database,
        runtime: options.runtime,
        media_path: options.media_path,
        destination_path: staging.path(),
        mode: BackupMode::Archive,
    })
    .await?;
    archive::write_tar_gz(staging.path(), options.destination_path)?;
    Ok(manifest)
}

async fn export_directory_backup(
    options: BackupExportOptions<'_>,
) -> Result<BackupManifest, BackupError> {
    archive::ensure_empty_or_absent(options.destination_path)?;
    fs::create_dir_all(options.destination_path.join("db"))?;

    let manifest = match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let resolved = sqlite::resolved_sqlite_options(connect_options, options.runtime);
            let pool = sqlx::SqlitePool::connect_with(resolved).await?;
            sqlite::backup::export_database(&pool, options.destination_path, options.mode).await?
        }
        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = postgres::resolved_postgres_options(pg_options, options.runtime);
            let pool = sqlx::PgPool::connect_with(resolved).await?;
            postgres::backup::export_database(&pool, options.destination_path, options.mode).await?
        }
    };

    let previous_backup = media::previous_directory_backup(options.destination_path)?;
    media::mirror_media_directory(
        options.media_path,
        &options.destination_path.join("media"),
        previous_backup.as_deref(),
    )?;
    let content_root = options.media_path.parent().ok_or_else(|| {
        BackupError::InvalidBackup("media storage path has no content-root parent".into())
    })?;
    media::mirror_media_directory(
        &content_root.join("themes"),
        &options.destination_path.join("themes"),
        previous_backup
            .as_deref()
            .map(|path| path.join("themes"))
            .as_deref(),
    )?;
    format::write_manifest(options.destination_path, &manifest)?;
    Ok(manifest)
}

async fn preflight_directory_backup(
    options: BackupRestoreOptions<'_>,
    manifest: &BackupManifest,
) -> Result<(), BackupError> {
    if !options.source_path.join("db").is_dir() {
        return Err(BackupError::InvalidBackup(format!(
            "missing db directory: {}",
            options.source_path.join("db").display()
        )));
    }

    match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let resolved = sqlite::resolved_sqlite_options(connect_options, options.runtime);
            let pool = sqlx::SqlitePool::connect_with(resolved).await?;
            sqlite::backup::preflight_restore_database(&pool, options.source_path, manifest).await
        }
        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = postgres::resolved_postgres_options(pg_options, options.runtime);
            let pool = sqlx::PgPool::connect_with(resolved).await?;
            postgres::backup::preflight_restore_database(&pool, options.source_path, manifest).await
        }
    }
}

async fn restore_directory_backup(
    options: BackupRestoreOptions<'_>,
    manifest: &BackupManifest,
) -> Result<RestoreValidationReport, BackupError> {
    if !options.source_path.join("db").is_dir() {
        return Err(BackupError::InvalidBackup(format!(
            "missing db directory: {}",
            options.source_path.join("db").display()
        )));
    }

    match options.database {
        DbConnectOptions::Sqlite(connect_options) => {
            let resolved = sqlite::resolved_sqlite_options(connect_options, options.runtime);
            let pool = sqlx::SqlitePool::connect_with(resolved).await?;
            sqlite::backup::restore_database(&pool, options.source_path, manifest).await
        }

        DbConnectOptions::Postgres {
            options: pg_options,
            ..
        } => {
            let resolved = postgres::resolved_postgres_options(pg_options, options.runtime);
            let pool = sqlx::PgPool::connect_with(resolved).await?;
            postgres::backup::restore_database(&pool, options.source_path, manifest).await
        }
    }
}
/// Ensures every restored serving identity has its exact immutable bytes before
/// database import can make that identity observable.
fn validate_theme_content_backup(source_path: &Path) -> Result<(), BackupError> {
    let charge_rows = source_path
        .join("db")
        .join("theme_retained_content_charges.ndjson");
    let mut lengths = BTreeMap::new();
    if charge_rows.exists() {
        for line in BufReader::new(fs::File::open(charge_rows)?).lines() {
            let row: serde_json::Value = serde_json::from_str(&line?)?;
            let (Some(digest), Some(length)) = (
                row.get("digest").and_then(serde_json::Value::as_str),
                row.get("physical_bytes")
                    .and_then(serde_json::Value::as_i64),
            ) else {
                return Err(BackupError::InvalidBackup(
                    "invalid retained theme content charge".into(),
                ));
            };
            if length < 0
                || lengths
                    .insert(digest.to_owned(), length)
                    .is_some_and(|prior| prior != length)
            {
                return Err(BackupError::InvalidBackup(
                    "inconsistent retained theme content charge".into(),
                ));
            }
        }
    }
    let rows = source_path
        .join("db")
        .join("theme_content_eligibility.ndjson");
    if !rows.exists() {
        return Ok(());
    }
    for line in BufReader::new(fs::File::open(rows)?).lines() {
        let row: serde_json::Value = serde_json::from_str(&line?)?;
        let digest = row
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .filter(|value| {
                value.len() == 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
            })
            .ok_or_else(|| BackupError::InvalidBackup("invalid theme content digest".into()))?;
        let mime = row
            .get("mime")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| BackupError::InvalidBackup("theme content MIME is missing".into()))?;
        if !matches!(
            mime,
            "text/css; charset=utf-8"
                | "font/woff2"
                | "image/png"
                | "image/jpeg"
                | "image/webp"
                | "image/avif"
        ) {
            return Err(BackupError::InvalidBackup(
                "invalid theme content MIME".into(),
            ));
        }
        let bytes = fs::read(
            source_path
                .join("themes")
                .join(&digest[..2])
                .join(&digest[2..4])
                .join(digest),
        )
        .map_err(|_| BackupError::InvalidBackup(format!("missing theme content {digest}")))?;
        if lengths
            .get(digest)
            .is_some_and(|expected| *expected != i64::try_from(bytes.len()).unwrap_or(-1))
        {
            return Err(BackupError::InvalidBackup(format!(
                "invalid theme content length {digest}"
            )));
        }
        let computed = Sha256::digest(&bytes);
        let mut actual = String::with_capacity(computed.len() * 2);
        for byte in computed {
            let _ = write!(actual, "{byte:02x}");
        }
        if actual != digest {
            return Err(BackupError::InvalidBackup(format!(
                "corrupt theme content {digest}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use std::{fmt::Write as _, fs, path::Path, sync::Arc};

    use crate::{
        StorageRuntimeConfig, ThemeAssetManager, ThemeOwner,
        test_support::{
            Backend, backends, compiled_theme_fixture, confirmed, create_site_theme,
            recorded_postgres_url, sqlite_url, theme_quota_limits,
        },
    };
    use common::theme::ThemeContentDigest;
    use rstest::*;
    use rstest_reuse::*;

    use super::{
        BackupExportOptions, BackupMode, BackupRestoreOptions, export_backup, restore_backup,
    };

    fn backup_database_options(
        backend: Backend,
        base: &tempfile::TempDir,
    ) -> crate::DbConnectOptions {
        match backend {
            Backend::Sqlite => sqlite_url(base),
            Backend::Postgres => recorded_postgres_url(base)
                .parse()
                .expect("recorded postgres URL parses"),
        }
    }

    fn theme_digest(bytes: &[u8]) -> ThemeContentDigest {
        let digest = Sha256::digest(bytes);
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            let _ = write!(hex, "{byte:02x}");
        }
        hex.parse().expect("SHA-256 is a valid theme digest")
    }

    fn theme_content_path(content_root: &Path, digest: &str) -> std::path::PathBuf {
        content_root
            .join("themes")
            .join(&digest[..2])
            .join(&digest[2..4])
            .join(digest)
    }

    async fn export_published_theme_backup(
        backend: Backend,
        source: &crate::test_support::TestEnv,
    ) -> (
        std::path::PathBuf,
        host::theme_package::CompiledThemeRevision,
        crate::ThemeRevision,
    ) {
        let source_database = backup_database_options(backend, &source.base);
        let source_media_path = source.base.path().join("media");
        fs::create_dir_all(&source_media_path).expect("create source media directory");
        let compiled = compiled_theme_fixture();
        let theme_id = create_site_theme(
            Arc::clone(&source.state.themes),
            source.state.write_scope.clone(),
            &compiled,
        )
        .await;
        let content_bytes = compiled
            .css()
            .bytes()
            .len()
            .checked_add(compiled.assets().map(|(_, _, bytes, _)| bytes.len()).sum())
            .and_then(|bytes| i64::try_from(bytes).ok())
            .expect("fixture content bytes fit");
        let manager = ThemeAssetManager::new(
            Arc::clone(&source.state.themes),
            source.state.write_scope.clone(),
            Arc::new(source.base.path().to_path_buf()),
        );
        let revision = confirmed(
            manager
                .publish(
                    ThemeOwner::Site,
                    theme_id,
                    &compiled,
                    theme_quota_limits(content_bytes),
                    100,
                )
                .await
                .expect("publish fixture theme"),
        );
        let backup = source.base.path().join("backup");
        let runtime = StorageRuntimeConfig::default();
        export_backup(BackupExportOptions {
            database: &source_database,
            runtime: &runtime,
            media_path: &source_media_path,
            destination_path: &backup,
            mode: BackupMode::Directory,
        })
        .await
        .expect("export published theme backup");
        (backup, compiled, revision)
    }

    #[apply(backends)]
    #[tokio::test]
    async fn restore_rejects_missing_eligible_theme_content_before_database_import(
        #[case] backend: Backend,
    ) {
        let source = backend.setup().await;
        let (backup, compiled, _) = export_published_theme_backup(backend, &source).await;
        let stylesheet_digest = theme_digest(compiled.css().bytes());
        fs::remove_file(theme_content_path(&backup, stylesheet_digest.as_ref()))
            .expect("remove backup stylesheet");

        let target = backend.setup().await;
        let target_database = backup_database_options(backend, &target.base);
        let target_media_path = target.base.path().join("media");
        fs::create_dir_all(&target_media_path).expect("create target media directory");
        let runtime = StorageRuntimeConfig::default();
        assert!(
            restore_backup(BackupRestoreOptions {
                database: &target_database,
                runtime: &runtime,
                media_path: &target_media_path,
                source_path: &backup,
            })
            .await
            .is_err()
        );
        assert!(
            target
                .state
                .themes
                .list_content_eligibility()
                .await
                .expect("read target eligibility")
                .is_empty()
        );
        assert!(
            target
                .state
                .themes
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read target catalog")
                .is_empty()
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn restore_rejects_corrupt_eligible_theme_content_before_database_import(
        #[case] backend: Backend,
    ) {
        let source = backend.setup().await;
        let (backup, compiled, _) = export_published_theme_backup(backend, &source).await;
        let stylesheet_digest = theme_digest(compiled.css().bytes());
        fs::write(
            theme_content_path(&backup, stylesheet_digest.as_ref()),
            b"corrupt theme stylesheet",
        )
        .expect("corrupt backup stylesheet");

        let target = backend.setup().await;
        let target_database = backup_database_options(backend, &target.base);
        let target_media_path = target.base.path().join("media");
        fs::create_dir_all(&target_media_path).expect("create target media directory");
        let runtime = StorageRuntimeConfig::default();
        assert!(
            restore_backup(BackupRestoreOptions {
                database: &target_database,
                runtime: &runtime,
                media_path: &target_media_path,
                source_path: &backup,
            })
            .await
            .is_err()
        );
        assert!(
            target
                .state
                .themes
                .list_content_eligibility()
                .await
                .expect("read target eligibility")
                .is_empty()
        );
        assert!(
            target
                .state
                .themes
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read target catalog")
                .is_empty()
        );
    }
    #[apply(backends)]
    #[tokio::test]
    async fn restore_schema_rejection_leaves_target_theme_content_unchanged(
        #[case] backend: Backend,
    ) {
        let source = backend.setup().await;
        let (backup, _, _) = export_published_theme_backup(backend, &source).await;
        let manifest_path = backup.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
                .expect("parse manifest");
        manifest["schema_version"] = serde_json::json!(0);
        fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write incompatible manifest");

        let target = backend.setup().await;
        let target_theme_path = target.base.path().join("themes").join("sentinel");
        fs::create_dir_all(target_theme_path.parent().expect("sentinel parent"))
            .expect("create target theme fixture");
        fs::write(&target_theme_path, b"target theme content").expect("write target content");
        let target_database = backup_database_options(backend, &target.base);
        let target_media_path = target.base.path().join("media");
        fs::create_dir_all(&target_media_path).expect("create target media directory");
        let runtime = StorageRuntimeConfig::default();

        assert!(
            restore_backup(BackupRestoreOptions {
                database: &target_database,
                runtime: &runtime,
                media_path: &target_media_path,
                source_path: &backup,
            })
            .await
            .is_err()
        );
        assert_eq!(
            fs::read(target_theme_path).expect("read unchanged target content"),
            b"target theme content"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn restore_row_rejection_leaves_target_theme_content_unchanged(#[case] backend: Backend) {
        let source = backend.setup().await;
        let (backup, _, _) = export_published_theme_backup(backend, &source).await;
        fs::write(backup.join("db").join("site_config.ndjson"), "null\n")
            .expect("write invalid backup row");

        let target = backend.setup().await;
        let target_theme_path = target.base.path().join("themes").join("sentinel");
        fs::create_dir_all(target_theme_path.parent().expect("sentinel parent"))
            .expect("create target theme fixture");
        fs::write(&target_theme_path, b"target theme content").expect("write target content");
        let target_database = backup_database_options(backend, &target.base);
        let target_media_path = target.base.path().join("media");
        fs::create_dir_all(&target_media_path).expect("create target media directory");
        let runtime = StorageRuntimeConfig::default();

        assert!(
            restore_backup(BackupRestoreOptions {
                database: &target_database,
                runtime: &runtime,
                media_path: &target_media_path,
                source_path: &backup,
            })
            .await
            .is_err()
        );
        assert_eq!(
            fs::read(target_theme_path).expect("read unchanged target content"),
            b"target theme content"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn full_backup_restore_preserves_theme_rows_and_immutable_content(
        #[case] backend: Backend,
    ) {
        let source = backend.setup().await;
        let (backup, compiled, expected_revision) =
            export_published_theme_backup(backend, &source).await;
        let expected_catalog = source
            .state
            .themes
            .list_themes(ThemeOwner::Site)
            .await
            .expect("read source catalog");
        let expected_draft = source
            .state
            .themes
            .get_draft(ThemeOwner::Site, expected_revision.theme_id)
            .await
            .expect("read source draft")
            .expect("source draft exists");
        let expected_eligibility = source
            .state
            .themes
            .list_content_eligibility()
            .await
            .expect("read source eligibility");
        let expected_owner_quota = source
            .state
            .themes
            .owner_quota(ThemeOwner::Site)
            .await
            .expect("read source owner quota");
        let expected_site_quota = source
            .state
            .themes
            .site_quota()
            .await
            .expect("read source site quota");
        let charge_query = "SELECT catalog_owner_key, digest, CAST(logical_bytes AS TEXT), CAST(physical_bytes AS TEXT), CAST(live_references AS TEXT) FROM theme_retained_content_charges ORDER BY catalog_owner_key, digest";
        let expected_charges = source
            .base
            .pool()
            .string_quintuples(charge_query)
            .await
            .expect("read source content charges");
        let asset_query = "SELECT CAST(theme_id AS TEXT), revision_digest, path, digest, mime FROM theme_revision_assets ORDER BY theme_id, revision_digest, path";
        let expected_assets = source
            .base
            .pool()
            .string_quintuples(asset_query)
            .await
            .expect("read source revision assets");

        let target = backend.setup().await;
        let target_database = backup_database_options(backend, &target.base);
        let target_media_path = target.base.path().join("media");
        fs::create_dir_all(&target_media_path).expect("create target media directory");
        let runtime = StorageRuntimeConfig::default();
        restore_backup(BackupRestoreOptions {
            database: &target_database,
            runtime: &runtime,
            media_path: &target_media_path,
            source_path: &backup,
        })
        .await
        .expect("restore published theme backup");

        assert_eq!(
            target
                .state
                .themes
                .list_themes(ThemeOwner::Site)
                .await
                .expect("read restored catalog"),
            expected_catalog
        );
        assert_eq!(
            target
                .state
                .themes
                .get_draft(ThemeOwner::Site, expected_revision.theme_id)
                .await
                .expect("read restored draft"),
            Some(expected_draft)
        );
        assert_eq!(
            target
                .state
                .themes
                .list_revisions(ThemeOwner::Site, expected_revision.theme_id)
                .await
                .expect("read restored revisions"),
            vec![expected_revision]
        );
        assert_eq!(
            target
                .base
                .pool()
                .string_quintuples(asset_query)
                .await
                .expect("read restored revision assets"),
            expected_assets
        );
        assert_eq!(
            target
                .state
                .themes
                .list_content_eligibility()
                .await
                .expect("read restored eligibility"),
            expected_eligibility
        );
        assert_eq!(
            target
                .base
                .pool()
                .string_quintuples(charge_query)
                .await
                .expect("read restored content charges"),
            expected_charges
        );
        assert_eq!(
            target
                .state
                .themes
                .owner_quota(ThemeOwner::Site)
                .await
                .expect("read restored owner quota"),
            expected_owner_quota
        );
        assert_eq!(
            target
                .state
                .themes
                .site_quota()
                .await
                .expect("read restored site quota"),
            expected_site_quota
        );

        let stylesheet_digest = theme_digest(compiled.css().bytes());
        assert_eq!(
            fs::read(theme_content_path(
                target.base.path(),
                stylesheet_digest.as_ref()
            ))
            .expect("read restored stylesheet"),
            compiled.css().bytes()
        );
        for (_, _, bytes, _) in compiled.assets() {
            let digest = theme_digest(bytes);
            assert_eq!(
                fs::read(theme_content_path(target.base.path(), digest.as_ref()))
                    .expect("read restored theme asset"),
                bytes
            );
        }
    }
}

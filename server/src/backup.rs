//! Background backup worker subsystem: the scheduled job that exports the
//! database + media to the configured destination, plus retention pruning.
//! Self-contained (no router coupling); split out of the crate root per §1.7.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_cron_scheduler::{Job, JobScheduler};

use common::backup::BackupConfig;
use storage::{export_backup, AppState, BackupExportOptions, BackupMode, DbConnectOptions};

/// Starts the background backup worker if configured.
///
/// # Errors
///
/// Returns an error if the site configuration cannot be loaded, or if the
/// job scheduler fails to start.
pub async fn start_backup_worker(
    state: Arc<AppState>,
    database: DbConnectOptions,
    storage_path: PathBuf,
) -> anyhow::Result<Option<JobScheduler>> {
    let config = state.site_config.get_backup_config().await?;
    let Some(destination_root) = config.destination_path.as_deref().map(PathBuf::from) else {
        tracing::warn!("backup worker disabled: backup.destination_path is not configured");
        return Ok(None);
    };

    let scheduler = JobScheduler::new().await?;
    let schedule = config.schedule.as_str().to_owned();
    let job = Job::new_async(schedule.as_str(), move |_uuid, _lock| {
        let database = database.clone();
        let media_path = storage_path.join("media");
        let destination_root = destination_root.clone();
        let config = config.clone();
        Box::pin(async move {
            if let Err(error) =
                run_scheduled_backup(&database, &media_path, &destination_root, &config).await
            {
                tracing::error!(error = %error, "scheduled backup failed");
            }
        })
    })?;
    scheduler.add(job).await?;
    scheduler.start().await?;
    Ok(Some(scheduler))
}

async fn run_scheduled_backup(
    database: &DbConnectOptions,
    media_path: &Path,
    destination_root: &Path,
    config: &BackupConfig,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(destination_root)?;
    let destination_path = backup_path_for_mode(destination_root, config.mode);
    export_backup(BackupExportOptions {
        database,
        media_path,
        destination_path: &destination_path,
        mode: config.mode,
    })
    .await?;
    prune_backups(destination_root, config.retention_count)?;
    tracing::info!(path = %destination_path.display(), "scheduled backup complete");
    Ok(destination_path)
}

fn prune_backups(destination_root: &Path, retention_count: usize) -> std::io::Result<()> {
    let mut backups = Vec::new();
    if !destination_root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(destination_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.join("manifest.json").is_file()
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".tar.gz"))
        {
            backups.push(path);
        }
    }
    backups.sort();
    let prune_count = backups.len().saturating_sub(retention_count);
    for path in backups.into_iter().take(prune_count) {
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn timestamped_backup_name() -> String {
    format!("backup-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"))
}

fn backup_path_for_mode(destination_root: &Path, mode: BackupMode) -> PathBuf {
    let name = timestamped_backup_name();
    match mode {
        BackupMode::Directory => destination_root.join(name),
        BackupMode::Archive => destination_root.join(format!("{name}.tar.gz")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::{BACKUP_DESTINATION_PATH_KEY, BACKUP_SCHEDULE_KEY};
    use tempfile::TempDir;

    async fn test_state() -> Arc<AppState> {
        storage::open_database(&"sqlite::memory:".parse().unwrap())
            .await
            .unwrap()
    }

    #[test]
    fn timestamped_backup_name_has_expected_format() {
        let name = timestamped_backup_name();
        assert!(
            name.starts_with("backup-"),
            "name must start with 'backup-', got: {name}"
        );
        let suffix = name.strip_prefix("backup-").unwrap();
        // Format: YYYYMMDDTHHMMSSz (16 chars)
        assert_eq!(
            suffix.len(),
            16,
            "timestamp suffix must be 16 chars, got: {suffix}"
        );
        assert!(suffix.ends_with('Z'), "timestamp must end with 'Z'");
        assert!(suffix.contains('T'), "timestamp must contain 'T'");
    }

    #[tokio::test]
    async fn backup_worker_disabled_without_destination_path() {
        let state = test_state().await;
        let storage = TempDir::new().expect("temp dir");
        let scheduler = start_backup_worker(
            state,
            "sqlite::memory:".parse().expect("sqlite options"),
            storage.path().to_path_buf(),
        )
        .await
        .expect("worker start");

        assert!(scheduler.is_none());
    }

    #[tokio::test]
    async fn backup_worker_starts_when_destination_is_configured() {
        let state = test_state().await;
        let storage = TempDir::new().expect("temp dir");
        state
            .site_config
            .set(
                BACKUP_DESTINATION_PATH_KEY,
                storage.path().join("backups").to_str().expect("utf-8 path"),
            )
            .await
            .expect("set destination");
        state
            .site_config
            .set(BACKUP_SCHEDULE_KEY, "0 0 0 1 1 *")
            .await
            .expect("set schedule");

        let scheduler = start_backup_worker(
            state,
            "sqlite::memory:".parse().expect("sqlite options"),
            storage.path().to_path_buf(),
        )
        .await
        .expect("worker start");

        assert!(scheduler.is_some());
    }

    #[tokio::test]
    async fn backup_worker_executes_scheduled_backup() {
        let temp = TempDir::new().expect("temp dir");
        let db_options: DbConnectOptions =
            format!("sqlite:{}", temp.path().join("jaunder.db").display())
                .parse()
                .expect("db options");
        let state = storage::open_database(&db_options).await.expect("open db");
        let storage_path = temp.path().join("storage");
        let media_path = storage_path.join("media");
        std::fs::create_dir_all(&media_path).expect("media dir");
        std::fs::write(media_path.join("file.txt"), "media").expect("media file");
        let destination_path = temp.path().join("scheduled-backups");
        state
            .site_config
            .set(
                BACKUP_DESTINATION_PATH_KEY,
                destination_path.to_str().expect("utf-8 path"),
            )
            .await
            .expect("set destination");
        state
            .site_config
            .set(BACKUP_SCHEDULE_KEY, "*/1 * * * * *")
            .await
            .expect("set schedule");

        let mut scheduler = start_backup_worker(state, db_options, storage_path)
            .await
            .expect("worker start")
            .expect("scheduler enabled");

        let mut found_manifest = false;
        for _ in 0..30 {
            found_manifest = std::fs::read_dir(&destination_path)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .any(|entry| entry.path().join("manifest.json").is_file());
            if found_manifest {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        scheduler.shutdown().await.expect("shutdown scheduler");

        assert!(found_manifest, "scheduled backup did not run");
    }

    #[tokio::test]
    async fn run_scheduled_backup_writes_backup_and_prunes_old_ones() {
        let temp = TempDir::new().expect("temp dir");
        let db_url = format!("sqlite:{}", temp.path().join("jaunder.db").display());
        storage::open_database(&db_url.parse().expect("db options"))
            .await
            .expect("open db");

        let media = temp.path().join("media");
        std::fs::create_dir(&media).expect("media dir");
        std::fs::write(media.join("file.txt"), "media").expect("media file");

        let destination_root = temp.path().join("backups");
        for name in ["backup-0001", "backup-0002"] {
            let backup = destination_root.join(name);
            std::fs::create_dir_all(&backup).expect("old backup dir");
            std::fs::write(backup.join("manifest.json"), "{}").expect("manifest");
        }

        let config = BackupConfig {
            destination_path: Some(destination_root.to_string_lossy().into_owned()),
            schedule: common::backup::BackupSchedule::parse("0 0 0 1 1 *").expect("valid schedule"),
            retention_count: 1,
            mode: BackupMode::Directory,
        };
        let written = run_scheduled_backup(
            &db_url.parse().expect("db options"),
            &media,
            &destination_root,
            &config,
        )
        .await
        .expect("scheduled backup");

        assert!(written.join("manifest.json").is_file());
        assert!(written.join("media").join("file.txt").is_file());
        assert!(!destination_root.join("backup-0001").exists());
        assert!(!destination_root.join("backup-0002").exists());
    }

    #[test]
    fn prune_backups_keeps_newest_manifest_directories() {
        let temp = TempDir::new().expect("temp dir");
        for name in ["backup-1", "backup-2", "backup-3"] {
            let path = temp.path().join(name);
            std::fs::create_dir(&path).expect("backup dir");
            std::fs::write(path.join("manifest.json"), "{}").expect("manifest");
        }
        let ignored = temp.path().join("not-a-backup");
        std::fs::create_dir(&ignored).expect("ignored dir");

        prune_backups(temp.path(), 2).expect("prune");

        assert!(!temp.path().join("backup-1").exists());
        assert!(temp.path().join("backup-2").exists());
        assert!(temp.path().join("backup-3").exists());
        assert!(ignored.exists());
    }

    #[test]
    fn prune_backups_keeps_newest_archives() {
        let temp = TempDir::new().expect("temp dir");
        for name in ["backup-1.tar.gz", "backup-2.tar.gz", "backup-3.tar.gz"] {
            std::fs::write(temp.path().join(name), "archive").expect("archive");
        }

        prune_backups(temp.path(), 2).expect("prune");

        assert!(!temp.path().join("backup-1.tar.gz").exists());
        assert!(temp.path().join("backup-2.tar.gz").exists());
        assert!(temp.path().join("backup-3.tar.gz").exists());
    }

    #[test]
    fn prune_backups_accepts_missing_destination_root() {
        let temp = TempDir::new().expect("temp dir");
        prune_backups(&temp.path().join("missing"), 1).expect("prune missing root");
    }

    #[test]
    fn backup_path_for_mode_returns_tar_gz_for_archive_mode() {
        let root = std::path::Path::new("/backups");
        let path = backup_path_for_mode(root, BackupMode::Archive);
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            name.ends_with(".tar.gz"),
            "expected .tar.gz extension, got: {name}"
        );
        assert!(
            name.starts_with("backup-"),
            "expected backup- prefix, got: {name}"
        );
    }
}

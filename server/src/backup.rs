//! Background backup worker subsystem: the scheduled job that exports the
//! database + media to the configured destination, plus retention pruning.
//! Self-contained (no router coupling); split out of the crate root per §1.7.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use flate2::read::GzDecoder;
use tokio_cron_scheduler::{Job, JobScheduler};

use common::backup::BackupConfig;
use common::time::UtcInstant;
use storage::{
    BackupExportOptions, BackupManifest, BackupMode, DbConnectOptions, SiteConfigStorage,
    StorageRuntimeConfig, export_backup,
};

/// Starts the background backup worker if configured.
///
/// # Errors
///
/// Returns an error if the site configuration cannot be loaded, or if the
/// job scheduler fails to start.
pub async fn start_backup_worker(
    site_config: Arc<dyn SiteConfigStorage>,
    database: DbConnectOptions,
    runtime: StorageRuntimeConfig,
    storage_path: PathBuf,
) -> anyhow::Result<Option<JobScheduler>> {
    let config = site_config.get_backup_config().await?;
    let Some(destination_root) = config.destination_path.as_deref().map(PathBuf::from) else {
        tracing::warn!("backup worker disabled: backup.destination_path is not configured");
        return Ok(None);
    };

    let scheduler = JobScheduler::new().await?;
    let schedule = config.schedule.to_string();
    let job = Job::new_async(schedule.as_str(), move |_uuid, _lock| {
        let database = database.clone();
        let runtime = runtime.clone();
        let media_path = storage_path.join("media");
        let destination_root = destination_root.clone();
        let config = config.clone();
        Box::pin(async move {
            run_scheduled_backup_logged(
                &database,
                &runtime,
                &media_path,
                &destination_root,
                &config,
            )
            .await;
        })
    })?;
    scheduler.add(job).await?;
    scheduler.start().await?;
    Ok(Some(scheduler))
}

async fn run_scheduled_backup(
    database: &DbConnectOptions,
    runtime: &StorageRuntimeConfig,
    media_path: &Path,
    destination_root: &Path,
    config: &BackupConfig,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(destination_root)?;
    let destination_path = backup_path_for_mode(destination_root, config.mode);
    export_backup(BackupExportOptions {
        database,
        runtime,
        media_path,
        destination_path: &destination_path,
        mode: config.mode,
    })
    .await?;
    let pruned = prune_backups(destination_root, config.retention_count.value())?;
    host::metrics::backup_bytes(backup_size_bytes(&destination_path));
    host::metrics::backup_pruned(u64::try_from(pruned).unwrap_or(u64::MAX));
    tracing::info!(path = %destination_path.display(), "scheduled backup complete");
    Ok(destination_path)
}

/// Total on-disk size of a backup artifact: the file length for an archive, or
/// the recursive sum of readable files for a directory backup. Measurement is
/// secondary to the successful backup, so unreadable entries contribute zero
/// and are reported once for the whole aggregate.
fn backup_size_bytes(path: &Path) -> u64 {
    let (size, error) = backup_size_result(path);
    finish_backup_size(size, error)
}

type MetadataOperation = fn(&Path) -> std::io::Result<std::fs::Metadata>;
type ReadDirOperation = fn(&Path) -> std::io::Result<std::fs::ReadDir>;

fn read_metadata(path: &Path) -> std::io::Result<std::fs::Metadata> {
    fs::metadata(path)
}

fn read_directory(path: &Path) -> std::io::Result<std::fs::ReadDir> {
    fs::read_dir(path)
}

fn backup_size_result(path: &Path) -> (u64, Option<std::io::Error>) {
    backup_size_result_with(path, read_metadata, read_directory)
}

fn backup_size_result_with(
    path: &Path,
    metadata: MetadataOperation,
    read_dir: ReadDirOperation,
) -> (u64, Option<std::io::Error>) {
    match metadata(path) {
        Ok(metadata_value) if metadata_value.is_file() => (metadata_value.len(), None),
        Ok(_) => {
            let entries = match read_dir(path) {
                Ok(entries) => entries,
                Err(error) => return (0, Some(error)),
            };
            backup_size_from_entries(
                entries.map(|entry| entry.map(|entry| entry.path())),
                |child| backup_size_result_with(child, metadata, read_dir),
            )
        }
        Err(error) => (0, Some(error)),
    }
}

fn backup_size_from_entries<I>(
    entries: I,
    mut measure: impl FnMut(&Path) -> (u64, Option<std::io::Error>),
) -> (u64, Option<std::io::Error>)
where
    I: IntoIterator<Item = std::io::Result<PathBuf>>,
{
    let mut size = 0_u64;
    let mut first_error = None;
    for entry in entries {
        match entry {
            Ok(path) => {
                let (entry_size, error) = measure(&path);
                size = size.saturating_add(entry_size);
                if first_error.is_none() {
                    first_error = error;
                }
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    (size, first_error)
}

fn finish_backup_size(size: u64, error: Option<std::io::Error>) -> u64 {
    if let Some(error) = error {
        host::error::report_swallowed(
            host::error::ErrorKind::Internal,
            host::error::ErrorClass::Transient,
            "server.backup.measure_size",
            host::error::SwallowedSource::Error(&error),
        );
    }
    size
}

/// Runs one scheduled backup and logs any failure, swallowing the error so a
/// transient backup failure never tears the scheduler down. Extracted from the
/// job closure so both the success and failure paths can be exercised directly
/// from a test, rather than depending on the cron scheduler firing (whose
/// background-thread execution is awkward to observe under coverage).
async fn run_scheduled_backup_logged(
    database: &DbConnectOptions,
    runtime: &StorageRuntimeConfig,
    media_path: &Path,
    destination_root: &Path,
    config: &BackupConfig,
) {
    let started = std::time::Instant::now();
    let result =
        run_scheduled_backup(database, runtime, media_path, destination_root, config).await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    host::metrics::backup_duration_ms(elapsed_ms);
    host::metrics::backup_run(backup_result_metric(result.is_ok()));
    if let Err(error) = result {
        host::error::report_swallowed(
            host::error::ErrorKind::Storage,
            host::error::ErrorClass::Transient,
            "server.backup.scheduled_run",
            host::error::SwallowedSource::Error(error.as_ref()),
        );
    }
}

/// Maps a backup run's success flag to its bounded `result` attribute.
fn backup_result_metric(succeeded: bool) -> host::metrics::BackupResult {
    if succeeded {
        host::metrics::BackupResult::Success
    } else {
        host::metrics::BackupResult::Failure
    }
}

/// Removes all but the newest `retention_count` backups under
/// `destination_root`, returning the number pruned.
fn prune_backups(destination_root: &Path, retention_count: usize) -> std::io::Result<usize> {
    let mut backups = Vec::new();
    if !destination_root.exists() {
        return Ok(0);
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
    Ok(prune_count)
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

/// Returns the newest successful backup manifest timestamp under `destination_root`.
///
/// Malformed candidate artifacts are skipped and reported once with a fixed
/// metrics context. Missing destination roots are treated as no successful backup.
///
/// # Errors
///
/// Returns an error when the destination root exists but cannot be enumerated.
pub fn latest_successful_backup_timestamp(
    destination_root: &Path,
) -> anyhow::Result<Option<UtcInstant>> {
    if !destination_root.exists() {
        return Ok(None);
    }

    let mut latest: Option<UtcInstant> = None;
    let mut saw_malformed_artifact = false;
    for entry in fs::read_dir(destination_root)? {
        let path = entry?.path();
        let timestamp = if path.join("manifest.json").is_file() {
            read_directory_backup_timestamp(&path)
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".tar.gz"))
        {
            read_archive_backup_timestamp(&path)
        } else {
            continue;
        };

        match timestamp {
            Ok(timestamp) => {
                latest = Some(latest.map_or(timestamp, |current| current.max(timestamp)));
            }
            Err(_) => saw_malformed_artifact = true,
        }
    }

    if saw_malformed_artifact {
        host::error::report_swallowed(
            host::error::ErrorKind::Storage,
            host::error::ErrorClass::Transient,
            "server.metrics.backup_last_success",
            host::error::SwallowedSource::Redacted,
        );
    }

    Ok(latest)
}

fn read_directory_backup_timestamp(path: &Path) -> anyhow::Result<UtcInstant> {
    let manifest = fs::read_to_string(path.join("manifest.json"))?;
    let manifest: BackupManifest = serde_json::from_str(&manifest)?;
    Ok(manifest.timestamp)
}

fn read_archive_backup_timestamp(path: &Path) -> anyhow::Result<UtcInstant> {
    let file = File::open(path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if archive_entry_is_manifest(&entry.path()?) {
            let mut manifest = String::new();
            entry.read_to_string(&mut manifest)?;
            let manifest: BackupManifest = serde_json::from_str(&manifest)?;
            return Ok(manifest.timestamp);
        }
    }
    anyhow::bail!("archive backup manifest missing")
}

fn archive_entry_is_manifest(path: &Path) -> bool {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value),
            _ => None,
        })
        .eq([std::ffi::OsStr::new("manifest.json")])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{migrated_sqlite_db, site_config};
    use common::test_support::{parse_destination_path, parse_retention_count};
    use host::config_key::SiteConfigKey;
    use tempfile::TempDir;
    #[derive(Clone)]
    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("trace lock").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    fn trace_capture() -> (
        tracing::subscriber::DefaultGuard,
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        (tracing::subscriber::set_default(subscriber), output)
    }

    fn trace_text(output: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        String::from_utf8(output.lock().expect("trace lock").clone()).expect("utf8 trace")
    }

    fn assert_one_report(trace: &str, context: &str) {
        assert_eq!(
            trace.matches(r#""error.disposition":"swallowed""#).count(),
            1,
            "trace: {trace}"
        );
        assert!(
            trace.contains(&format!(r#""error.context":"{context}""#)),
            "trace: {trace}"
        );
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

    fn test_manifest(timestamp: &str, mode: BackupMode) -> BackupManifest {
        BackupManifest {
            version: "0.1.0".to_owned(),
            schema_version: 1,
            schema_checksum: "test".to_owned(),
            timestamp: timestamp.parse().expect("test timestamp"),
            mode,
            tables: Vec::new(),
        }
    }

    fn write_test_manifest(path: &Path, timestamp: &str, mode: BackupMode) {
        std::fs::create_dir_all(path).expect("backup dir");
        let manifest = serde_json::to_vec(&test_manifest(timestamp, mode)).expect("manifest json");
        std::fs::write(path.join("manifest.json"), manifest).expect("write manifest");
    }

    fn write_test_archive(path: &Path, timestamp: &str) {
        let staging = TempDir::new().expect("archive staging");
        write_test_manifest(staging.path(), timestamp, BackupMode::Archive);
        let file = File::create(path).expect("archive file");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        archive
            .append_dir_all(".", staging.path())
            .expect("append archive");
        let encoder = archive.into_inner().expect("finish archive");
        encoder.finish().expect("finish gzip");
    }

    fn write_test_archive_without_manifest(path: &Path) {
        let staging = TempDir::new().expect("archive staging");
        std::fs::write(staging.path().join("notes.txt"), b"not a manifest").expect("notes");
        let file = File::create(path).expect("archive file");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        archive
            .append_dir_all(".", staging.path())
            .expect("append archive");
        let encoder = archive.into_inner().expect("finish archive");
        encoder.finish().expect("finish gzip");
    }

    #[test]
    fn latest_successful_backup_timestamp_uses_directory_manifest_timestamp() {
        let temp = TempDir::new().expect("tempdir");
        let older = temp.path().join("backup-20260101T000000Z");
        let newer = temp.path().join("backup-20260102T000000Z");
        write_test_manifest(&older, "2026-01-01T00:00:00Z", BackupMode::Directory);
        write_test_manifest(&newer, "2026-01-02T00:00:00Z", BackupMode::Directory);

        let timestamp = latest_successful_backup_timestamp(temp.path())
            .expect("timestamp scan")
            .expect("timestamp");

        assert_eq!(timestamp.value().to_rfc3339(), "2026-01-02T00:00:00+00:00");
    }

    #[test]
    fn latest_successful_backup_timestamp_reads_archive_manifest_timestamp() {
        let temp = TempDir::new().expect("tempdir");
        write_test_archive(
            &temp.path().join("backup-20260102T000000Z.tar.gz"),
            "2026-01-02T00:00:00Z",
        );

        let timestamp = latest_successful_backup_timestamp(temp.path())
            .expect("timestamp scan")
            .expect("timestamp");

        assert_eq!(timestamp.value().to_rfc3339(), "2026-01-02T00:00:00+00:00");
    }

    #[test]
    fn latest_successful_backup_timestamp_ignores_suffix_only_archive() {
        let temp = TempDir::new().expect("tempdir");
        std::fs::write(
            temp.path().join("backup-20260102T000000Z.tar.gz"),
            b"not a tar",
        )
        .expect("write bad archive");

        let (guard, output) = trace_capture();
        let timestamp = latest_successful_backup_timestamp(temp.path()).expect("timestamp scan");
        drop(guard);

        assert_eq!(timestamp, None);
        assert_one_report(&trace_text(&output), "server.metrics.backup_last_success");
    }

    #[test]
    fn latest_successful_backup_timestamp_ignores_archive_without_manifest() {
        let temp = TempDir::new().expect("tempdir");
        write_test_archive_without_manifest(&temp.path().join("backup-20260102T000000Z.tar.gz"));

        let (guard, output) = trace_capture();
        let timestamp = latest_successful_backup_timestamp(temp.path()).expect("timestamp scan");
        drop(guard);

        assert_eq!(timestamp, None);
        assert_one_report(&trace_text(&output), "server.metrics.backup_last_success");
    }

    #[test]
    fn latest_successful_backup_timestamp_ignores_malformed_manifest() {
        let temp = TempDir::new().expect("tempdir");
        let bad = temp.path().join("backup-20260102T000000Z");
        std::fs::create_dir(&bad).expect("dir");
        std::fs::write(bad.join("manifest.json"), b"{").expect("bad manifest");

        let (guard, output) = trace_capture();
        let timestamp = latest_successful_backup_timestamp(temp.path()).expect("timestamp scan");
        drop(guard);

        assert_eq!(timestamp, None);
        assert_one_report(&trace_text(&output), "server.metrics.backup_last_success");
    }

    #[test]
    fn latest_successful_backup_timestamp_accepts_missing_destination_root() {
        let temp = TempDir::new().expect("tempdir");

        let timestamp = latest_successful_backup_timestamp(&temp.path().join("missing"))
            .expect("timestamp scan");

        assert_eq!(timestamp, None);
    }

    #[test]
    fn latest_successful_backup_timestamp_ignores_non_backup_entries() {
        let temp = TempDir::new().expect("tempdir");
        std::fs::write(temp.path().join("notes.txt"), b"ignore me").expect("notes");

        let timestamp = latest_successful_backup_timestamp(temp.path()).expect("timestamp scan");

        assert_eq!(timestamp, None);
    }

    #[tokio::test]
    async fn backup_worker_disabled_without_destination_path() {
        let storage = TempDir::new().expect("temp dir");
        let (db, pool) = migrated_sqlite_db(storage.path()).await;
        let scheduler = start_backup_worker(
            site_config(&pool),
            db,
            StorageRuntimeConfig::default(),
            storage.path().to_path_buf(),
        )
        .await
        .expect("worker start");

        assert!(scheduler.is_none());
    }

    #[tokio::test]
    async fn backup_worker_starts_when_destination_is_configured() {
        let storage = TempDir::new().expect("temp dir");
        let (db, pool) = migrated_sqlite_db(storage.path()).await;
        let cfg = site_config(&pool);
        cfg.set(
            SiteConfigKey::BackupDestinationPath,
            storage.path().join("backups").to_str().expect("utf-8 path"),
        )
        .await
        .expect("set destination");
        cfg.set(SiteConfigKey::BackupSchedule, "0 0 0 1 1 *")
            .await
            .expect("set schedule");

        let scheduler = start_backup_worker(
            cfg,
            db,
            StorageRuntimeConfig::default(),
            storage.path().to_path_buf(),
        )
        .await
        .expect("worker start");

        assert!(scheduler.is_some());
    }

    #[tokio::test]
    async fn backup_worker_executes_scheduled_backup() {
        let temp = TempDir::new().expect("temp dir");
        let (db, pool) = migrated_sqlite_db(temp.path()).await;
        let cfg = site_config(&pool);
        let storage_path = temp.path().join("storage");
        let media_path = storage_path.join("media");
        std::fs::create_dir_all(&media_path).expect("media dir");
        std::fs::write(media_path.join("file.txt"), "media").expect("media file");
        let destination_path = temp.path().join("scheduled-backups");
        cfg.set(
            SiteConfigKey::BackupDestinationPath,
            destination_path.to_str().expect("utf-8 path"),
        )
        .await
        .expect("set destination");
        cfg.set(SiteConfigKey::BackupSchedule, "*/1 * * * * *")
            .await
            .expect("set schedule");

        let mut scheduler =
            start_backup_worker(cfg, db, StorageRuntimeConfig::default(), storage_path)
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
        // `run_scheduled_backup` opens its own connection via `db`; we just need
        // the schema in place, so the migrated pool is dropped immediately.
        let (db, _pool) = migrated_sqlite_db(temp.path()).await;

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
            destination_path: Some(parse_destination_path(&destination_root.to_string_lossy())),
            schedule: "0 0 0 1 1 *".parse().expect("valid schedule"),
            retention_count: parse_retention_count("1"),
            mode: BackupMode::Directory,
        };
        let written = run_scheduled_backup(
            &db,
            &StorageRuntimeConfig::default(),
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

    #[tokio::test]
    async fn run_scheduled_backup_logged_runs_success_path_and_swallows_errors() {
        let temp = TempDir::new().expect("temp dir");
        let (db, _pool) = migrated_sqlite_db(temp.path()).await;
        let media = temp.path().join("media");
        std::fs::create_dir(&media).expect("media dir");

        // Success: a writable destination produces a backup directory.
        let ok_root = temp.path().join("ok-backups");
        let ok_config = BackupConfig {
            destination_path: Some(parse_destination_path(&ok_root.to_string_lossy())),
            schedule: "0 0 0 1 1 *".parse().expect("valid schedule"),
            retention_count: parse_retention_count("1"),
            mode: BackupMode::Directory,
        };
        run_scheduled_backup_logged(
            &db,
            &StorageRuntimeConfig::default(),
            &media,
            &ok_root,
            &ok_config,
        )
        .await;
        assert!(
            ok_root.exists(),
            "successful scheduled backup should create the destination"
        );

        // Failure: the destination's parent is a regular file, so create_dir_all
        // fails; the error must be logged and swallowed (no panic, nothing written).
        let blocker = temp.path().join("blocker");
        std::fs::write(&blocker, "x").expect("write blocker");
        let bad_root = blocker.join("backups");
        let bad_config = BackupConfig {
            destination_path: Some(parse_destination_path(&bad_root.to_string_lossy())),
            ..ok_config
        };
        let (guard, output) = trace_capture();
        run_scheduled_backup_logged(
            &db,
            &StorageRuntimeConfig::default(),
            &media,
            &bad_root,
            &bad_config,
        )
        .await;
        drop(guard);
        assert!(!bad_root.exists(), "failed scheduled backup writes nothing");
        assert_one_report(&trace_text(&output), "server.backup.scheduled_run");
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
    fn denied_metadata(_: &Path) -> std::io::Result<std::fs::Metadata> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "metadata denied",
        ))
    }

    fn denied_read_dir(_: &Path) -> std::io::Result<std::fs::ReadDir> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "read_dir denied",
        ))
    }

    #[test]
    fn continuation_reporting_backup_size_metadata_and_read_dir_failures_report_once() {
        let temp = TempDir::new().expect("temp dir");
        for ((size, error), message) in [
            (
                backup_size_result_with(temp.path(), denied_metadata, read_directory),
                "metadata denied",
            ),
            (
                backup_size_result_with(temp.path(), read_metadata, denied_read_dir),
                "read_dir denied",
            ),
        ] {
            assert_eq!(size, 0);
            let (guard, output) = trace_capture();
            assert_eq!(finish_backup_size(size, error), 0);
            drop(guard);
            let trace = trace_text(&output);
            assert_one_report(&trace, "server.backup.measure_size");
            assert!(trace.contains(message), "trace: {trace}");
        }
    }

    #[test]
    fn continuation_reporting_backup_size_iterator_and_child_failures_preserve_partial_size_and_first_error()
     {
        let entries = [
            Ok(PathBuf::from("readable")),
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "entry iteration denied",
            )),
            Ok(PathBuf::from("child-failure")),
        ];
        let (size, error) = backup_size_from_entries(entries, |path| {
            if path == Path::new("readable") {
                (5, None)
            } else {
                (
                    7,
                    Some(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "recursive child denied",
                    )),
                )
            }
        });
        assert_eq!(size, 12);
        let (guard, output) = trace_capture();
        assert_eq!(finish_backup_size(size, error), 12);
        drop(guard);
        let trace = trace_text(&output);
        assert_one_report(&trace, "server.backup.measure_size");
        assert!(trace.contains("entry iteration denied"), "trace: {trace}");
        assert!(!trace.contains("recursive child denied"), "trace: {trace}");
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
    fn backup_size_bytes_preserves_partial_size_and_reports_once() {
        let temp = TempDir::new().expect("temp dir");

        let (guard, output) = trace_capture();
        assert_eq!(backup_size_bytes(&temp.path().join("missing")), 0);
        drop(guard);
        assert_one_report(&trace_text(&output), "server.backup.measure_size");

        // A plain file reports its byte length.
        let file = temp.path().join("archive.tar.gz");
        std::fs::write(&file, b"hello").expect("write file");
        assert_eq!(backup_size_bytes(&file), 5);

        // A directory sums its contents recursively.
        let dir = temp.path().join("dir-backup");
        std::fs::create_dir_all(dir.join("media")).expect("create dirs");
        std::fs::write(dir.join("manifest.json"), b"abc").expect("write manifest");
        std::fs::write(dir.join("media").join("a.bin"), b"defg").expect("write media");
        assert_eq!(backup_size_bytes(&dir), 7);
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

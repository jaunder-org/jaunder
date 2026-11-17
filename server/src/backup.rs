//! The backup runner and its triggers: the scheduled worker, the operator's
//! manual **Back Up Now**, and the final backup taken during graceful shutdown.
//! Every trigger runs the same job — export the database + media to the saved
//! destination, then prune to the retention count — one run at a time.
//! Self-contained (no router coupling); split out of the crate root per §1.7.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::scheduled_worker::{ScheduledWorkerGuard, WorkTracker};
use async_trait::async_trait;
use flate2::read::GzDecoder;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_cron_scheduler::{Job, JobScheduler};

use common::backup::BackupConfig;
use common::time::UtcInstant;
use host::{error, metrics};
use storage::{
    BackupExportOptions, BackupManifest, BackupMode, DbConnectOptions, SiteConfigStorage,
    StorageRuntimeConfig,
};
use web::backup::{ManualBackupError, ManualBackupRunner};

/// What started a backup run; it names the run in logs and failure reports.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BackupTrigger {
    Scheduled,
    Shutdown,
    Manual,
}

impl BackupTrigger {
    fn name(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Shutdown => "shutdown",
            Self::Manual => "manual",
        }
    }

    fn failure_context(self) -> &'static str {
        match self {
            Self::Scheduled => "server.backup.scheduled_run",
            Self::Shutdown => "server.backup.shutdown_run",
            Self::Manual => "server.backup.manual_run",
        }
    }
}

/// Exclusive right to run a backup. Export already serializes on the database
/// lock, but pruning and the manual trigger's refusal need the runner's own
/// guard, held across export *and* prune.
pub struct BackupRunSlot {
    _running: OwnedMutexGuard<()>,
}

/// The one backup job shared by every trigger. Each run reads the saved
/// settings when it starts, so destination, retention and mode changes apply
/// to the next run without a restart.
#[derive(Clone)]
pub struct BackupRunner {
    site_config: Arc<dyn SiteConfigStorage>,
    database: DbConnectOptions,
    runtime: StorageRuntimeConfig,
    media_path: PathBuf,
    running: Arc<Mutex<()>>,
}

impl BackupRunner {
    #[must_use]
    pub fn new(
        site_config: Arc<dyn SiteConfigStorage>,
        database: DbConnectOptions,
        runtime: StorageRuntimeConfig,
        storage_path: &Path,
    ) -> Self {
        Self {
            site_config,
            database,
            runtime,
            media_path: storage_path.join("media"),
            running: Arc::new(Mutex::new(())),
        }
    }

    /// Waits until no other backup is running and claims the run slot.
    pub async fn wait_for_run_slot(&self) -> BackupRunSlot {
        BackupRunSlot {
            _running: Arc::clone(&self.running).lock_owned().await,
        }
    }

    /// The backup settings as currently saved.
    async fn saved_config(&self) -> anyhow::Result<BackupConfig> {
        Ok(self.site_config.get_backup_config().await?)
    }

    /// Runs one backup for a trigger nobody awaits: waits its turn behind any
    /// backup in progress, skips when no destination is saved, and reports and
    /// swallows a failure so it never tears down the scheduler or the stop.
    pub(crate) async fn run_unattended(&self, trigger: BackupTrigger) {
        let _slot = self.wait_for_run_slot().await;
        match self.run_once(trigger).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                tracing::info!(
                    trigger = trigger.name(),
                    "backup skipped: no destination is saved"
                );
            }
            Err(error) => error::report_swallowed(
                error::ErrorKind::Storage,
                error::ErrorClass::Transient,
                trigger.failure_context(),
                error::SwallowedSource::Error(error.as_ref()),
            ),
        }
    }

    /// Runs one backup with the slot already held. `Ok(None)` means no
    /// destination is saved, so nothing was attempted.
    async fn run_once(&self, trigger: BackupTrigger) -> anyhow::Result<Option<PathBuf>> {
        let config = self.saved_config().await?;
        let Some(destination_root) = config.destination_path.as_deref().map(PathBuf::from) else {
            return Ok(None);
        };
        let started = std::time::Instant::now();
        let result = self.write_backup(&destination_root, &config, trigger).await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        metrics::backup_duration_ms(elapsed_ms);
        metrics::backup_run(backup_result_metric(result.is_ok()));
        result.map(Some)
    }

    async fn write_backup(
        &self,
        destination_root: &Path,
        config: &BackupConfig,
        trigger: BackupTrigger,
    ) -> anyhow::Result<PathBuf> {
        fs::create_dir_all(destination_root)?;
        let destination_path = unused_backup_path(destination_root, config.mode).await;
        storage::export_backup(BackupExportOptions {
            database: &self.database,
            runtime: &self.runtime,
            media_path: &self.media_path,
            destination_path: &destination_path,
            mode: config.mode,
        })
        .await?;
        let pruned = prune_backups(destination_root, config.retention_count.value())?;
        metrics::backup_bytes(backup_size_bytes(&destination_path));
        metrics::backup_pruned(u64::try_from(pruned).unwrap_or(u64::MAX));
        tracing::info!(
            path = %destination_path.display(),
            trigger = trigger.name(),
            "backup complete"
        );
        Ok(destination_path)
    }
}

#[async_trait]
impl ManualBackupRunner for BackupRunner {
    /// Refuses rather than queues when another backup is running: the operator
    /// is waiting on this request, and the run in progress already captures the
    /// current state.
    async fn run_manual_backup(&self) -> Result<String, ManualBackupError> {
        let slot = Arc::clone(&self.running)
            .try_lock_owned()
            .map_err(|_| ManualBackupError::AlreadyRunning)?;
        let runner = self.clone();
        // The job runs on its own task so a client disconnect, which drops this
        // request future, cannot abandon a half-written backup.
        let job = tokio::spawn(async move {
            let _slot = slot;
            runner.run_once(BackupTrigger::Manual).await
        });
        match job
            .await
            .map_err(|error| ManualBackupError::Failed(error.into()))?
        {
            Ok(Some(path)) => Ok(path.display().to_string()),
            Ok(None) => Err(ManualBackupError::NoDestination),
            Err(error) => Err(ManualBackupError::Failed(
                error
                    .context(BackupTrigger::Manual.failure_context())
                    .into(),
            )),
        }
    }
}

/// Starts the scheduled backup worker when a destination is saved at startup.
/// The cron expression and the worker's enablement are fixed here; each run
/// still reads the rest of the saved settings when it starts.
///
/// # Errors
///
/// Returns an error if the site configuration cannot be loaded, or if the
/// job scheduler fails to start.
pub(crate) async fn start_backup_worker(
    runner: BackupRunner,
) -> anyhow::Result<Option<ScheduledWorkerGuard>> {
    let config = runner.saved_config().await?;
    if config.destination_path.is_none() {
        tracing::warn!("backup worker disabled: backup.destination_path is not configured");
        return Ok(None);
    }

    let scheduler = JobScheduler::new().await?;
    let tracker = WorkTracker::default();
    let job_tracker = tracker.clone();
    let schedule = config.schedule.to_string();
    let job = Job::new_async(schedule.as_str(), move |_uuid, _lock| {
        let runner = runner.clone();
        let tracker = job_tracker.clone();
        Box::pin(tracker.run(async move {
            runner.run_unattended(BackupTrigger::Scheduled).await;
        }))
    })?;
    scheduler.add(job).await?;
    ScheduledWorkerGuard::start(scheduler, tracker)
        .await
        .map(Some)
}

/// A timestamped backup path not already taken. Names have one-second
/// resolution and export refuses an existing path; because runs are
/// serialized, a taken name only means a run finished earlier this second, so
/// waiting for the clock to move on (at most a second) yields a fresh name.
async fn unused_backup_path(destination_root: &Path, mode: BackupMode) -> PathBuf {
    loop {
        let path = backup_path_for_mode(destination_root, mode);
        if !path.exists() {
            return path;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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
        error::report_swallowed(
            error::ErrorKind::Internal,
            error::ErrorClass::Transient,
            "server.backup.measure_size",
            error::SwallowedSource::Error(&error),
        );
    }
    size
}

/// Maps a backup run's success flag to its bounded `result` attribute.
fn backup_result_metric(succeeded: bool) -> metrics::BackupResult {
    if succeeded {
        metrics::BackupResult::Success
    } else {
        metrics::BackupResult::Failure
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
    format!(
        "backup-{}",
        UtcInstant::now().value().strftime("%Y%m%dT%H%M%SZ")
    )
}

pub(crate) fn backup_path_for_mode(destination_root: &Path, mode: BackupMode) -> PathBuf {
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
        error::report_swallowed(
            error::ErrorKind::Storage,
            error::ErrorClass::Transient,
            "server.metrics.backup_last_success",
            error::SwallowedSource::Redacted,
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
    use host::config_key::SiteConfigKey;
    use storage::test_support::{confirmed, sqlite_write_scope};
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
            format_version: 1,
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

        assert_eq!(timestamp, "2026-01-02T00:00:00Z".parse().unwrap());
        assert_eq!(timestamp.to_string(), "2026-01-02T00:00:00.000000000Z");
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

        assert_eq!(timestamp, "2026-01-02T00:00:00Z".parse().unwrap());
        assert_eq!(timestamp.to_string(), "2026-01-02T00:00:00.000000000Z");
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

    /// Saves one site-config value through the production write path.
    async fn save_setting(
        pool: &sqlx::SqlitePool,
        cfg: &Arc<dyn SiteConfigStorage>,
        key: SiteConfigKey,
        value: &str,
    ) {
        let config = Arc::clone(cfg);
        let value = value.to_owned();
        confirmed(
            sqlite_write_scope(pool.clone())
                .run(move |transaction| {
                    Box::pin(async move { config.set(transaction, key, &value).await })
                })
                .await
                .expect("save setting"),
        );
    }

    /// A runner over a fresh migrated database whose storage directory holds
    /// one media file, plus the pool and site config for saving settings.
    struct RunnerFixture {
        temp: TempDir,
        db: DbConnectOptions,
        pool: sqlx::SqlitePool,
        site_config: Arc<dyn SiteConfigStorage>,
        runner: BackupRunner,
    }

    impl RunnerFixture {
        async fn new() -> Self {
            let temp = TempDir::new().expect("temp dir");
            let (db, pool) = migrated_sqlite_db(temp.path()).await;
            let storage_path = temp.path().join("storage");
            let media_path = storage_path.join("media");
            std::fs::create_dir_all(&media_path).expect("media dir");
            std::fs::write(media_path.join("file.txt"), "media").expect("media file");
            let site_config = site_config(&pool);
            let runner = BackupRunner::new(
                Arc::clone(&site_config),
                db.clone(),
                StorageRuntimeConfig::default(),
                &storage_path,
            );
            Self {
                temp,
                db,
                pool,
                site_config,
                runner,
            }
        }

        async fn save(&self, key: SiteConfigKey, value: &str) {
            save_setting(&self.pool, &self.site_config, key, value).await;
        }

        async fn save_destination(&self, destination: &Path) {
            self.save(
                SiteConfigKey::BackupDestinationPath,
                destination.to_str().expect("utf-8 path"),
            )
            .await;
        }
    }

    fn manifest_backups(root: &Path) -> Vec<PathBuf> {
        let mut backups: Vec<PathBuf> = std::fs::read_dir(root)
            .expect("read destination")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.join("manifest.json").is_file())
            .collect();
        backups.sort();
        backups
    }

    #[tokio::test]
    async fn backup_worker_disabled_without_destination_path() {
        let fixture = RunnerFixture::new().await;
        let scheduler = start_backup_worker(fixture.runner.clone())
            .await
            .expect("worker start");

        assert!(scheduler.is_none());
    }

    #[tokio::test]
    async fn backup_worker_starts_when_destination_is_configured() {
        let fixture = RunnerFixture::new().await;
        fixture
            .save_destination(&fixture.temp.path().join("backups"))
            .await;
        fixture
            .save(SiteConfigKey::BackupSchedule, "0 0 0 1 1 *")
            .await;

        let scheduler = start_backup_worker(fixture.runner.clone())
            .await
            .expect("worker start");

        assert!(scheduler.is_some());
    }

    #[tokio::test]
    async fn backup_worker_executes_scheduled_backup() {
        let fixture = RunnerFixture::new().await;
        let destination_path = fixture.temp.path().join("scheduled-backups");
        fixture.save_destination(&destination_path).await;
        fixture
            .save(SiteConfigKey::BackupSchedule, "*/1 * * * * *")
            .await;

        let mut scheduler = start_backup_worker(fixture.runner.clone())
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
    async fn manual_backup_writes_backup_and_prunes_to_retention() {
        let fixture = RunnerFixture::new().await;
        let destination_root = fixture.temp.path().join("backups");
        for name in ["backup-0001", "backup-0002"] {
            let backup = destination_root.join(name);
            std::fs::create_dir_all(&backup).expect("old backup dir");
            std::fs::write(backup.join("manifest.json"), "{}").expect("manifest");
        }
        fixture.save_destination(&destination_root).await;
        fixture.save(SiteConfigKey::BackupRetentionCount, "1").await;

        let written = PathBuf::from(
            fixture
                .runner
                .run_manual_backup()
                .await
                .expect("manual backup"),
        );

        assert!(written.join("manifest.json").is_file());
        assert!(written.join("media").join("file.txt").is_file());
        assert_eq!(manifest_backups(&destination_root), vec![written]);
    }

    #[tokio::test]
    async fn back_to_back_runs_within_one_second_get_distinct_names() {
        let fixture = RunnerFixture::new().await;
        let destination_root = fixture.temp.path().join("backups");
        fixture.save_destination(&destination_root).await;

        let first = fixture.runner.run_manual_backup().await.expect("first");
        let second = fixture.runner.run_manual_backup().await.expect("second");

        assert_ne!(first, second);
        assert_eq!(
            manifest_backups(&destination_root),
            vec![PathBuf::from(first), PathBuf::from(second)]
        );
    }

    #[tokio::test]
    async fn unused_backup_path_waits_for_the_next_second_when_the_name_is_taken() {
        let temp = TempDir::new().expect("temp dir");
        let taken = backup_path_for_mode(temp.path(), BackupMode::Directory);
        std::fs::create_dir(&taken).expect("taken name");

        let fresh = unused_backup_path(temp.path(), BackupMode::Directory).await;

        assert_ne!(fresh, taken);
        assert!(!fresh.exists());
    }

    #[tokio::test]
    async fn each_run_reads_settings_saved_after_the_runner_was_built() {
        let fixture = RunnerFixture::new().await;
        assert!(matches!(
            fixture.runner.run_manual_backup().await,
            Err(ManualBackupError::NoDestination)
        ));

        let destination_root = fixture.temp.path().join("later-backups");
        fixture.save_destination(&destination_root).await;
        fixture.save(SiteConfigKey::BackupMode, "archive").await;
        let written = fixture.runner.run_manual_backup().await.expect("backup");

        assert!(written.ends_with(".tar.gz"), "written: {written}");
        assert!(Path::new(&written).starts_with(&destination_root));
        assert!(Path::new(&written).is_file());
    }

    #[tokio::test]
    async fn manual_backup_is_refused_while_another_run_holds_the_slot() {
        let fixture = RunnerFixture::new().await;
        fixture
            .save_destination(&fixture.temp.path().join("backups"))
            .await;
        let slot = fixture.runner.wait_for_run_slot().await;

        let refused = fixture.runner.run_manual_backup().await;

        assert!(matches!(refused, Err(ManualBackupError::AlreadyRunning)));
        drop(slot);
        fixture.runner.run_manual_backup().await.expect("free slot");
    }

    #[tokio::test]
    async fn unattended_run_waits_for_the_slot_instead_of_skipping() {
        let fixture = RunnerFixture::new().await;
        let destination_root = fixture.temp.path().join("backups");
        fixture.save_destination(&destination_root).await;
        let slot = fixture.runner.wait_for_run_slot().await;
        let runner = fixture.runner.clone();
        let mut waiting = tokio::spawn(async move {
            runner.run_unattended(BackupTrigger::Shutdown).await;
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiting)
                .await
                .is_err(),
            "an unattended run must wait while the slot is held"
        );

        drop(slot);
        waiting.await.expect("unattended run");

        assert_eq!(manifest_backups(&destination_root).len(), 1);
    }

    #[tokio::test]
    async fn interrupted_archive_run_leaves_nothing_pruning_counts() {
        let fixture = RunnerFixture::new().await;
        let destination_root = fixture.temp.path().join("backups");
        std::fs::create_dir_all(&destination_root).expect("destination");
        let destination = backup_path_for_mode(&destination_root, BackupMode::Archive);

        storage::export_backup_paused_before_archive_publish(
            BackupExportOptions {
                database: &fixture.db,
                runtime: &StorageRuntimeConfig::default(),
                media_path: &fixture.temp.path().join("storage").join("media"),
                destination_path: &destination,
                mode: BackupMode::Archive,
            },
            || {
                // A run killed here leaves only its work directory behind.
                assert!(!destination.exists());
                assert_eq!(prune_backups(&destination_root, 0).expect("prune"), 0);
                assert_eq!(
                    latest_successful_backup_timestamp(&destination_root).expect("scan"),
                    None
                );
            },
        )
        .await
        .expect("archive export");

        assert!(destination.is_file());
    }

    #[tokio::test]
    async fn unattended_runs_skip_without_destination_and_report_failures_by_trigger() {
        let fixture = RunnerFixture::new().await;
        let (guard, output) = trace_capture();
        fixture
            .runner
            .run_unattended(BackupTrigger::Scheduled)
            .await;
        drop(guard);
        let trace = trace_text(&output);
        assert!(trace.contains("backup skipped"), "trace: {trace}");
        assert!(!trace.contains("swallowed"), "trace: {trace}");

        // The destination's parent is a regular file, so create_dir_all fails;
        // the error is reported under the trigger's context and swallowed.
        let blocker = fixture.temp.path().join("blocker");
        std::fs::write(&blocker, "x").expect("write blocker");
        let bad_root = blocker.join("backups");
        fixture.save_destination(&bad_root).await;
        for trigger in [BackupTrigger::Scheduled, BackupTrigger::Shutdown] {
            let (guard, output) = trace_capture();
            fixture.runner.run_unattended(trigger).await;
            drop(guard);
            assert_one_report(&trace_text(&output), trigger.failure_context());
        }
        assert!(!bad_root.exists(), "failed backup writes nothing");
    }

    #[tokio::test]
    async fn manual_backup_failure_is_returned_not_swallowed() {
        let fixture = RunnerFixture::new().await;
        let blocker = fixture.temp.path().join("blocker");
        std::fs::write(&blocker, "x").expect("write blocker");
        fixture.save_destination(&blocker.join("backups")).await;

        let (guard, output) = trace_capture();
        let result = fixture.runner.run_manual_backup().await;
        drop(guard);

        let Err(ManualBackupError::Failed(error)) = result else {
            panic!("a failed manual backup must return its failure");
        };
        assert_eq!(error.to_string(), "server.backup.manual_run");
        assert!(!trace_text(&output).contains("swallowed"));
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

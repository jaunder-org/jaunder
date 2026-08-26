//! Serve-owned saturation metric sampling.
//!
//! OpenTelemetry observable callbacks are synchronous, so this module keeps
//! async storage and filesystem reads in a sampler task that updates a small
//! in-memory snapshot. The server composition root injects only the storage
//! handles and service facts each source needs; the sampler never owns the
//! whole [`storage::AppState`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use host::metrics::SaturationSnapshot;
use storage::{DbPoolObserver, DbPoolSnapshot, FeedEventStorage, MediaStorage};
use tokio::task::JoinHandle;

use crate::backup::latest_successful_backup_timestamp;

const FEED_CLAIM_LEASE_TIMEOUT: chrono::Duration = chrono::Duration::minutes(5);
const SATURATION_SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

fn measure_media_filesystem_bytes(root: &Path) -> io::Result<u64> {
    measure_media_filesystem_bytes_with(root, read_media_metadata, read_media_directory)
}

fn read_media_metadata(path: &Path) -> io::Result<fs::Metadata> {
    fs::symlink_metadata(path)
}

fn read_media_directory(path: &Path) -> io::Result<fs::ReadDir> {
    fs::read_dir(path)
}

fn measure_media_filesystem_bytes_with(
    root: &Path,
    metadata: fn(&Path) -> io::Result<fs::Metadata>,
    read_dir: fn(&Path) -> io::Result<fs::ReadDir>,
) -> io::Result<u64> {
    let root_metadata = metadata(root)?;
    if root_metadata.file_type().is_symlink() {
        return Err(unexpected_media_filesystem_entry(root, "symbolic link"));
    }
    if !root_metadata.is_dir() {
        return Err(unexpected_media_filesystem_entry(
            root,
            "non-directory root",
        ));
    }
    measure_media_directory_bytes(root, metadata, read_dir)
}

fn measure_media_directory_bytes(
    directory: &Path,
    metadata: fn(&Path) -> io::Result<fs::Metadata>,
    read_dir: fn(&Path) -> io::Result<fs::ReadDir>,
) -> io::Result<u64> {
    let mut bytes: u64 = 0;
    for entry in read_dir(directory)? {
        let path = entry?.path();
        let entry_metadata = metadata(&path)?;
        let entry_bytes = if entry_metadata.file_type().is_symlink() {
            return Err(unexpected_media_filesystem_entry(&path, "symbolic link"));
        } else if entry_metadata.is_file() {
            entry_metadata.len()
        } else if entry_metadata.is_dir() {
            measure_media_directory_bytes(&path, metadata, read_dir)?
        } else {
            return Err(unexpected_media_filesystem_entry(&path, "special entry"));
        };
        bytes = bytes
            .checked_add(entry_bytes)
            .ok_or_else(|| io::Error::other("media filesystem usage exceeds u64"))?;
    }
    Ok(bytes)
}

fn unexpected_media_filesystem_entry(path: &Path, kind: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("media filesystem contains {kind}: {}", path.display()),
    )
}
#[derive(Clone)]
struct MediaFilesystemSource {
    root: PathBuf,
    measure: fn(&Path) -> io::Result<u64>,
}

impl MediaFilesystemSource {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            measure: measure_media_filesystem_bytes,
        }
    }

    #[cfg(test)]
    fn with_measurement(root: PathBuf, measure: fn(&Path) -> io::Result<u64>) -> Self {
        Self { root, measure }
    }

    async fn sample(&self) -> anyhow::Result<u64> {
        let root = self.root.clone();
        let measure = self.measure;
        tokio::task::spawn_blocking(move || measure(&root))
            .await
            .map_err(anyhow::Error::from)?
            .map_err(anyhow::Error::from)
    }
}

#[derive(Clone)]
pub struct SaturationSources {
    inner: SaturationSourcesInner,
}

#[derive(Clone)]
enum SaturationSourcesInner {
    Real(RealSaturationSources),
    #[cfg(test)]
    Fake(FakeSaturationSources),
}

#[derive(Clone)]
struct RealSaturationSources {
    feed_events: Arc<dyn FeedEventStorage>,
    media: Arc<dyn MediaStorage>,
    media_filesystem: MediaFilesystemSource,
    backup_destination_root: Option<PathBuf>,
    db_pool: DbPoolObserver,
}

enum DbPoolReading {
    Snapshot(DbPoolSnapshot),
    #[cfg(test)]
    Failed,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
struct SaturationSample {
    feed_queue_depth: u64,
    backup_last_success_timestamp: i64,
    db_pool: DbPoolSnapshot,
    media_storage_bytes: u64,
    media_filesystem_bytes: u64,
}

#[cfg(test)]
#[derive(Clone)]
struct FakeSaturationSources {
    sample: SaturationSample,
    backup_configured: bool,
    failure: Option<FakeSaturationFailure>,
    media_filesystem: Option<MediaFilesystemSource>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum FakeSaturationFailure {
    Feed,
    Backup,
    DbPool,
    Media,
    MediaFilesystem,
}

impl SaturationSources {
    #[must_use]
    pub fn real(
        feed_events: Arc<dyn FeedEventStorage>,
        media: Arc<dyn MediaStorage>,
        media_root: PathBuf,
        backup_destination_root: Option<PathBuf>,
        db_pool: DbPoolObserver,
    ) -> Self {
        Self {
            inner: SaturationSourcesInner::Real(RealSaturationSources {
                feed_events,
                media,
                media_filesystem: MediaFilesystemSource::new(media_root),
                backup_destination_root,
                db_pool,
            }),
        }
    }

    async fn feed_queue_depth(&self) -> anyhow::Result<u64> {
        match &self.inner {
            SaturationSourcesInner::Real(real) => real
                .feed_events
                .claimable_count(FEED_CLAIM_LEASE_TIMEOUT)
                .await
                .map_err(Into::into),
            #[cfg(test)]
            SaturationSourcesInner::Fake(fake) => fake.read_feed_queue_depth(),
        }
    }

    fn backup_last_success_timestamp(&self) -> anyhow::Result<Option<i64>> {
        match &self.inner {
            SaturationSourcesInner::Real(real) => {
                let Some(root) = &real.backup_destination_root else {
                    return Ok(None);
                };
                latest_successful_backup_timestamp(root)
                    .map(|timestamp| timestamp.map(|timestamp| timestamp.value().timestamp()))
            }
            #[cfg(test)]
            SaturationSourcesInner::Fake(fake) => fake.read_backup_last_success_timestamp(),
        }
    }

    fn db_pool(&self) -> DbPoolReading {
        match &self.inner {
            SaturationSourcesInner::Real(real) => DbPoolReading::Snapshot(real.db_pool.snapshot()),
            #[cfg(test)]
            SaturationSourcesInner::Fake(fake) => fake.read_db_pool(),
        }
    }

    async fn media_storage_bytes(&self) -> anyhow::Result<u64> {
        match &self.inner {
            SaturationSourcesInner::Real(real) => real
                .media
                .total_upload_bytes()
                .await
                .map(|total| u64::try_from(total.value()).unwrap_or(u64::MAX))
                .map_err(Into::into),
            #[cfg(test)]
            SaturationSourcesInner::Fake(fake) => fake.read_media_storage_bytes(),
        }
    }

    async fn media_filesystem_bytes(&self) -> anyhow::Result<u64> {
        match &self.inner {
            SaturationSourcesInner::Real(real) => real.media_filesystem.sample().await,
            #[cfg(test)]
            SaturationSourcesInner::Fake(fake) => fake.read_media_filesystem_bytes().await,
        }
    }

    #[cfg(test)]
    fn fake_success(sample: SaturationSample) -> Self {
        Self::fake(sample, true, None)
    }

    #[cfg(test)]
    fn fake_backup_unconfigured(sample: SaturationSample) -> Self {
        Self::fake(sample, false, None)
    }

    #[cfg(test)]
    fn fake_feed_failure(sample: SaturationSample) -> Self {
        Self::fake(sample, true, Some(FakeSaturationFailure::Feed))
    }

    #[cfg(test)]
    fn fake_backup_failure(sample: SaturationSample) -> Self {
        Self::fake(sample, true, Some(FakeSaturationFailure::Backup))
    }

    #[cfg(test)]
    fn fake_db_pool_failure(sample: SaturationSample) -> Self {
        Self::fake(sample, true, Some(FakeSaturationFailure::DbPool))
    }

    #[cfg(test)]
    fn fake_media_failure(sample: SaturationSample) -> Self {
        Self::fake(sample, true, Some(FakeSaturationFailure::Media))
    }

    #[cfg(test)]
    fn fake_media_filesystem_failure(sample: SaturationSample) -> Self {
        Self::fake(sample, true, Some(FakeSaturationFailure::MediaFilesystem))
    }

    #[cfg(test)]
    fn fake_with_media_filesystem(
        sample: SaturationSample,
        media_filesystem: MediaFilesystemSource,
    ) -> Self {
        Self {
            inner: SaturationSourcesInner::Fake(FakeSaturationSources {
                sample,
                backup_configured: true,
                failure: None,
                media_filesystem: Some(media_filesystem),
            }),
        }
    }

    #[cfg(test)]
    fn fake(
        sample: SaturationSample,
        backup_configured: bool,
        failure: Option<FakeSaturationFailure>,
    ) -> Self {
        Self {
            inner: SaturationSourcesInner::Fake(FakeSaturationSources {
                sample,
                backup_configured,
                failure,
                media_filesystem: None,
            }),
        }
    }
}

#[cfg(test)]
impl FakeSaturationSources {
    fn read_feed_queue_depth(&self) -> anyhow::Result<u64> {
        if matches!(self.failure, Some(FakeSaturationFailure::Feed)) {
            anyhow::bail!("feed queue depth failed");
        }
        Ok(self.sample.feed_queue_depth)
    }

    fn read_backup_last_success_timestamp(&self) -> anyhow::Result<Option<i64>> {
        if matches!(self.failure, Some(FakeSaturationFailure::Backup)) {
            anyhow::bail!("backup timestamp failed");
        }
        Ok(self
            .backup_configured
            .then_some(self.sample.backup_last_success_timestamp))
    }

    fn read_db_pool(&self) -> DbPoolReading {
        if matches!(self.failure, Some(FakeSaturationFailure::DbPool)) {
            return DbPoolReading::Failed;
        }
        DbPoolReading::Snapshot(self.sample.db_pool)
    }

    fn read_media_storage_bytes(&self) -> anyhow::Result<u64> {
        if matches!(self.failure, Some(FakeSaturationFailure::Media)) {
            anyhow::bail!("media storage bytes failed");
        }
        Ok(self.sample.media_storage_bytes)
    }

    async fn read_media_filesystem_bytes(&self) -> anyhow::Result<u64> {
        if matches!(self.failure, Some(FakeSaturationFailure::MediaFilesystem)) {
            anyhow::bail!("media filesystem bytes failed");
        }
        match &self.media_filesystem {
            Some(media_filesystem) => media_filesystem.sample().await,
            None => Ok(self.sample.media_filesystem_bytes),
        }
    }
}

pub async fn sample_saturation_once(
    sources: &SaturationSources,
    snapshot: &RwLock<SaturationSnapshot>,
) {
    let feed_queue_depth = sources.feed_queue_depth().await;
    let backup_last_success_timestamp = sources.backup_last_success_timestamp();
    let db_pool = sources.db_pool();
    let media_storage_bytes = sources.media_storage_bytes().await;
    let media_filesystem_bytes = sources.media_filesystem_bytes().await;
    let mut snapshot = snapshot.write().unwrap_or_else(PoisonError::into_inner);
    snapshot.feed_queue_depth = if let Ok(value) = feed_queue_depth {
        Some(value)
    } else {
        report_source_failure("server.metrics.feed_queue_depth");
        None
    };
    snapshot.backup_last_success_timestamp = if let Ok(value) = backup_last_success_timestamp {
        value
    } else {
        report_source_failure("server.metrics.backup_last_success");
        None
    };
    match db_pool {
        DbPoolReading::Snapshot(value) => {
            snapshot.db_pool_used = Some(value.used);
            snapshot.db_pool_idle = Some(value.idle);
            snapshot.db_pool_max = Some(value.max);
        }
        #[cfg(test)]
        DbPoolReading::Failed => {
            report_source_failure("server.metrics.db_pool");
            snapshot.db_pool_used = None;
            snapshot.db_pool_idle = None;
            snapshot.db_pool_max = None;
        }
    }
    snapshot.media_storage_bytes = if let Ok(value) = media_storage_bytes {
        Some(value)
    } else {
        report_source_failure("server.metrics.media_storage_bytes");
        None
    };
    snapshot.media_filesystem_bytes = match media_filesystem_bytes {
        Ok(value) => Some(value),
        Err(error) => {
            host::error::report_swallowed(
                host::error::ErrorKind::Storage,
                host::error::ErrorClass::Transient,
                "server.metrics.media_filesystem_bytes",
                host::error::SwallowedSource::Error(error.as_ref()),
            );
            None
        }
    };
}

#[must_use]
pub fn spawn_saturation_sampler(
    sources: SaturationSources,
    snapshot: Arc<RwLock<SaturationSnapshot>>,
) -> JoinHandle<()> {
    spawn_saturation_sampler_with_interval(sources, snapshot, SATURATION_SAMPLE_INTERVAL)
}

fn spawn_saturation_sampler_with_interval(
    sources: SaturationSources,
    snapshot: Arc<RwLock<SaturationSnapshot>>,
    interval_duration: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(interval_duration);
        loop {
            interval.tick().await;
            sample_saturation_once(&sources, &snapshot).await;
        }
    })
}

fn report_source_failure(context: &'static str) {
    host::error::report_swallowed(
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        context,
        host::error::SwallowedSource::Redacted,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{LazyLock, Mutex, OnceLock, mpsc};

    #[derive(Clone)]
    struct SharedWriter(Arc<std::sync::Mutex<Vec<u8>>>);

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
        Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        (tracing::subscriber::set_default(subscriber), output)
    }

    fn trace_text(output: &Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        std::io::Write::flush(&mut SharedWriter(output.clone())).expect("flush trace");
        String::from_utf8(output.lock().expect("trace lock").clone()).expect("utf8 trace")
    }

    #[test]
    fn media_filesystem_measurement_sums_nested_regular_entries() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path().join("media");
        std::fs::create_dir_all(root.join("cached/deep")).expect("nested directory");
        std::fs::create_dir(root.join("tmp")).expect("tmp directory");
        std::fs::write(root.join("upload"), b"upload").expect("upload file");
        std::fs::write(root.join("cached/deep/file"), b"cached").expect("cached file");
        std::fs::write(root.join("tmp/file"), b"tmp").expect("tmp file");

        assert_eq!(
            measure_media_filesystem_bytes(&root).expect("filesystem measurement"),
            15
        );
    }

    struct BlockingMeasurement {
        start_async: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    static BLOCKING_MEASUREMENT: OnceLock<BlockingMeasurement> = OnceLock::new();

    fn blocking_measurement(path: &Path) -> io::Result<u64> {
        if path == Path::new("fail") {
            return Err(io::Error::other("injected blocking measurement failure"));
        }
        let measurement = BLOCKING_MEASUREMENT.get().expect("blocking measurement");
        measurement
            .start_async
            .lock()
            .expect("start lock")
            .take()
            .expect("start sender")
            .send(())
            .expect("start async task");
        measurement.entered.send(()).expect("measurement entered");
        measurement
            .release
            .lock()
            .expect("release lock")
            .recv()
            .expect("measurement release");
        Ok(1)
    }

    struct SlowMeasurement {
        active: AtomicUsize,
        max_active: AtomicUsize,
        calls: AtomicUsize,
    }

    static SLOW_MEASUREMENT: LazyLock<SlowMeasurement> = LazyLock::new(|| SlowMeasurement {
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
    });

    fn slow_measurement(path: &Path) -> io::Result<u64> {
        if path == Path::new("fail") {
            return Err(io::Error::other("injected slow measurement failure"));
        }
        let measurement = &*SLOW_MEASUREMENT;
        let active = measurement.active.fetch_add(1, Ordering::SeqCst) + 1;
        measurement.max_active.fetch_max(active, Ordering::SeqCst);
        measurement.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(25));
        measurement.active.fetch_sub(1, Ordering::SeqCst);
        Ok(1)
    }

    #[test]
    fn media_filesystem_measurement_counts_hard_links_per_directory_entry() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path().join("media");
        std::fs::create_dir_all(root.join("cached")).expect("cached directory");
        let original = root.join("upload");
        std::fs::write(&original, b"upload").expect("upload file");
        std::fs::hard_link(&original, root.join("cached/reference")).expect("hard link");

        assert_eq!(
            measure_media_filesystem_bytes(&root).expect("filesystem measurement"),
            12
        );
    }

    #[test]
    fn media_filesystem_measurement_rejects_missing_root() {
        let temp = tempfile::TempDir::new().expect("tempdir");

        let error =
            measure_media_filesystem_bytes(&temp.path().join("missing")).expect_err("missing root");

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn media_filesystem_measurement_rejects_a_symbolic_link_root() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let target = temp.path().join("target");
        std::fs::create_dir(&target).expect("target directory");
        let root = temp.path().join("media");
        std::os::unix::fs::symlink(&target, &root).expect("symbolic link");

        let error = measure_media_filesystem_bytes(&root).expect_err("symbolic link root");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn media_filesystem_measurement_rejects_a_non_directory_root() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path().join("media");
        std::fs::write(&root, b"not a directory").expect("root file");

        let error = measure_media_filesystem_bytes(&root).expect_err("non-directory root");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn media_filesystem_measurement_rejects_symlinks_without_a_partial_result() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path().join("media");
        std::fs::create_dir(&root).expect("media directory");
        let regular = root.join("regular");
        std::fs::write(&regular, b"regular").expect("regular file");
        std::os::unix::fs::symlink(&regular, root.join("linked")).expect("symbolic link");

        let error = measure_media_filesystem_bytes(&root).expect_err("symbolic links are rejected");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[cfg(unix)]
    #[test]
    fn media_filesystem_measurement_rejects_special_entries() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path().join("media");
        std::fs::create_dir(&root).expect("media directory");
        let _socket = std::os::unix::net::UnixListener::bind(root.join("socket")).expect("socket");

        let error =
            measure_media_filesystem_bytes(&root).expect_err("special entries are rejected");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    fn denied_metadata(_: &std::path::Path) -> std::io::Result<std::fs::Metadata> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "metadata denied",
        ))
    }

    fn denied_read_dir(_: &std::path::Path) -> std::io::Result<std::fs::ReadDir> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "read_dir denied",
        ))
    }

    #[test]
    fn media_filesystem_measurement_propagates_metadata_failures() {
        let temp = tempfile::TempDir::new().expect("tempdir");

        let error =
            measure_media_filesystem_bytes_with(temp.path(), denied_metadata, read_media_directory)
                .expect_err("metadata failure");

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn media_filesystem_measurement_propagates_directory_read_failures() {
        let temp = tempfile::TempDir::new().expect("tempdir");

        let error =
            measure_media_filesystem_bytes_with(temp.path(), read_media_metadata, denied_read_dir)
                .expect_err("directory read failure");

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    fn sample() -> SaturationSample {
        SaturationSample {
            feed_queue_depth: 7,
            backup_last_success_timestamp: 1_800_000_001,
            db_pool: DbPoolSnapshot {
                used: 2,
                idle: 4,
                max: 6,
            },
            media_storage_bytes: 8192,
            media_filesystem_bytes: 12_345,
        }
    }

    fn seeded_snapshot() -> RwLock<SaturationSnapshot> {
        RwLock::new(SaturationSnapshot {
            feed_queue_depth: Some(99),
            backup_last_success_timestamp: Some(99),
            db_pool_used: Some(99),
            db_pool_idle: Some(99),
            db_pool_max: Some(99),
            media_storage_bytes: Some(99),
            media_filesystem_bytes: Some(99),
        })
    }

    fn read_snapshot(snapshot: &RwLock<SaturationSnapshot>) -> SaturationSnapshot {
        snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn assert_success_snapshot(snapshot: &SaturationSnapshot) {
        assert_eq!(snapshot.feed_queue_depth, Some(7));
        assert_eq!(snapshot.backup_last_success_timestamp, Some(1_800_000_001));
        assert_eq!(snapshot.db_pool_used, Some(2));
        assert_eq!(snapshot.db_pool_idle, Some(4));
        assert_eq!(snapshot.db_pool_max, Some(6));
        assert_eq!(snapshot.media_storage_bytes, Some(8192));
        assert_eq!(snapshot.media_filesystem_bytes, Some(12_345));
    }

    #[tokio::test]
    async fn saturation_sampler_updates_snapshot_on_success() {
        let sources = SaturationSources::fake_success(sample());
        let snapshot = RwLock::new(SaturationSnapshot::default());

        sample_saturation_once(&sources, &snapshot).await;

        assert_success_snapshot(&read_snapshot(&snapshot));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn filesystem_measurement_runs_on_blocking_work() {
        let (start_sender, start_receiver) = tokio::sync::oneshot::channel();
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        assert!(
            BLOCKING_MEASUREMENT
                .set(BlockingMeasurement {
                    start_async: Mutex::new(Some(start_sender)),
                    entered: entered_sender,
                    release: Mutex::new(release_receiver),
                })
                .is_ok(),
            "one blocking measurement"
        );
        let async_worker_progressed = Arc::new(AtomicBool::new(false));
        let progress = async_worker_progressed.clone();
        tokio::spawn(async move {
            start_receiver.await.expect("blocking measurement starts");
            progress.store(true, Ordering::SeqCst);
        });
        let releaser = std::thread::spawn(move || {
            entered_receiver.recv().expect("measurement enters");
            std::thread::sleep(Duration::from_millis(25));
            let progressed = async_worker_progressed.load(Ordering::SeqCst);
            release_sender.send(()).expect("release measurement");
            progressed
        });

        let source = MediaFilesystemSource::with_measurement(PathBuf::new(), blocking_measurement);

        assert_eq!(source.sample().await.expect("blocking measurement"), 1);
        assert!(
            releaser.join().expect("releaser thread"),
            "an async worker must progress while filesystem work is blocked"
        );

        let failed_source =
            MediaFilesystemSource::with_measurement(PathBuf::from("fail"), blocking_measurement);
        let error = failed_source
            .sample()
            .await
            .expect_err("injected blocking measurement failure");
        assert!(
            error
                .to_string()
                .contains("injected blocking measurement failure")
        );
    }

    #[tokio::test]
    async fn saturation_sampler_clears_filesystem_snapshot_after_measurement_failure() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let root = temp.path().join("media");
        std::fs::create_dir(&root).expect("media directory");
        std::fs::write(root.join("orphan"), b"filesystem").expect("orphan file");
        let sources = SaturationSources::fake_with_media_filesystem(
            sample(),
            MediaFilesystemSource::new(root.clone()),
        );
        let snapshot = RwLock::new(SaturationSnapshot::default());

        sample_saturation_once(&sources, &snapshot).await;

        assert_eq!(read_snapshot(&snapshot).media_filesystem_bytes, Some(10));
        std::fs::remove_dir_all(&root).expect("remove media directory");
        let (guard, output) = trace_capture();
        sample_saturation_once(&sources, &snapshot).await;
        drop(guard);

        let snapshot = read_snapshot(&snapshot);
        assert_eq!(snapshot.media_filesystem_bytes, None);
        assert_eq!(snapshot.media_storage_bytes, Some(8192));
        let trace = trace_text(&output);
        let collection_error = std::fs::symlink_metadata(&root)
            .expect_err("media root remains absent")
            .to_string();
        assert!(trace.contains("server.metrics.media_filesystem_bytes"));

        assert!(
            trace.contains(&format!(r#""error.source":"{collection_error}""#)),
            "{trace}"
        );
    }

    #[tokio::test]
    async fn saturation_sampler_never_overlaps_filesystem_measurements() {
        let measurement = &*SLOW_MEASUREMENT;

        let failed_source =
            MediaFilesystemSource::with_measurement(PathBuf::from("fail"), slow_measurement);
        let error = failed_source
            .sample()
            .await
            .expect_err("injected slow measurement failure");
        assert!(
            error
                .to_string()
                .contains("injected slow measurement failure")
        );
        let sources = SaturationSources::fake_with_media_filesystem(
            sample(),
            MediaFilesystemSource::with_measurement(PathBuf::new(), slow_measurement),
        );
        let snapshot = Arc::new(RwLock::new(SaturationSnapshot::default()));
        let handle =
            spawn_saturation_sampler_with_interval(sources, snapshot, Duration::from_millis(1));

        tokio::time::timeout(Duration::from_secs(1), async {
            while measurement.calls.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("multiple filesystem measurements");
        handle.abort();
        let _ = handle.await;

        assert_eq!(
            measurement.max_active.load(Ordering::SeqCst),
            1,
            "the next periodic sample must await the prior filesystem measurement"
        );
    }

    #[tokio::test]
    async fn saturation_sampler_clears_backup_timestamp_when_unconfigured() {
        let sources = SaturationSources::fake_backup_unconfigured(sample());
        let snapshot = seeded_snapshot();
        let (guard, output) = trace_capture();

        sample_saturation_once(&sources, &snapshot).await;
        drop(guard);

        let snapshot = read_snapshot(&snapshot);
        assert_eq!(snapshot.feed_queue_depth, Some(7));
        assert_eq!(snapshot.backup_last_success_timestamp, None);
        assert_eq!(snapshot.db_pool_used, Some(2));
        assert_eq!(snapshot.db_pool_idle, Some(4));
        assert_eq!(snapshot.db_pool_max, Some(6));
        assert_eq!(snapshot.media_storage_bytes, Some(8192));
        assert_eq!(snapshot.media_filesystem_bytes, Some(12_345));
        assert!(
            !trace_text(&output).contains("server.metrics.backup_last_success"),
            "unconfigured backup destination should not report a failure"
        );
    }

    #[tokio::test]
    async fn saturation_sampler_clears_only_failed_source() {
        let cases: [(&str, SaturationSources); 5] = [
            ("feed", SaturationSources::fake_feed_failure(sample())),
            ("backup", SaturationSources::fake_backup_failure(sample())),
            ("db_pool", SaturationSources::fake_db_pool_failure(sample())),
            ("media", SaturationSources::fake_media_failure(sample())),
            (
                "filesystem",
                SaturationSources::fake_media_filesystem_failure(sample()),
            ),
        ];

        for (failed, sources) in cases {
            let snapshot = seeded_snapshot();
            sample_saturation_once(&sources, &snapshot).await;
            let got = read_snapshot(&snapshot);

            assert_eq!(
                got.feed_queue_depth,
                (failed != "feed").then_some(7),
                "{failed}"
            );
            assert_eq!(
                got.backup_last_success_timestamp,
                (failed != "backup").then_some(1_800_000_001),
                "{failed}"
            );
            assert_eq!(
                got.db_pool_used,
                (failed != "db_pool").then_some(2),
                "{failed}"
            );
            assert_eq!(
                got.db_pool_idle,
                (failed != "db_pool").then_some(4),
                "{failed}"
            );
            assert_eq!(
                got.db_pool_max,
                (failed != "db_pool").then_some(6),
                "{failed}"
            );
            assert_eq!(
                got.media_storage_bytes,
                (failed != "media").then_some(8192),
                "{failed}"
            );
            assert_eq!(
                got.media_filesystem_bytes,
                (failed != "filesystem").then_some(12_345),
                "{failed}"
            );
        }
    }

    #[tokio::test]
    async fn saturation_sampler_reports_fixed_context_on_failure() {
        let cases: [(&str, SaturationSources); 5] = [
            (
                "server.metrics.feed_queue_depth",
                SaturationSources::fake_feed_failure(sample()),
            ),
            (
                "server.metrics.backup_last_success",
                SaturationSources::fake_backup_failure(sample()),
            ),
            (
                "server.metrics.db_pool",
                SaturationSources::fake_db_pool_failure(sample()),
            ),
            (
                "server.metrics.media_storage_bytes",
                SaturationSources::fake_media_failure(sample()),
            ),
            (
                "server.metrics.media_filesystem_bytes",
                SaturationSources::fake_media_filesystem_failure(sample()),
            ),
        ];

        for (context, sources) in cases {
            let snapshot = seeded_snapshot();
            let (guard, output) = trace_capture();
            sample_saturation_once(&sources, &snapshot).await;
            drop(guard);
            let trace = trace_text(&output);

            assert_eq!(
                trace.matches(r#""error.disposition":"swallowed""#).count(),
                1,
                "{context}: {trace}"
            );
            assert!(
                trace.contains(&format!(r#""error.context":"{context}""#)),
                "{context}: {trace}"
            );
        }
    }

    #[rstest::rstest]
    #[case::sqlite(storage::test_support::Backend::Sqlite)]
    #[tokio::test]
    async fn saturation_sampler_reads_real_sources(
        #[case] backend: storage::test_support::Backend,
    ) {
        let base = tempfile::TempDir::new().expect("tempdir");
        let options = match backend {
            storage::test_support::Backend::Sqlite => storage::test_support::sqlite_url(&base),
            storage::test_support::Backend::Postgres => {
                unreachable!("sqlite_only supplies only SQLite")
            }
        };
        let opened = storage::open_database_with_observer(&options)
            .await
            .expect("open database");
        opened
            .state
            .feed_events
            .enqueue(&storage::test_support::fp("/feed.rss"))
            .await
            .expect("enqueue feed event");
        let media_root = base.path().join("media");
        std::fs::create_dir_all(media_root.join("cached")).expect("cached media directory");
        std::fs::write(media_root.join("cached/orphan"), b"orphan").expect("orphan media file");
        let backup_root = base.path().join("backups");
        let backup = backup_root.join("backup-20260102T000000Z");
        std::fs::create_dir_all(&backup).expect("backup dir");
        let manifest = storage::BackupManifest {
            version: "0.1.0".to_owned(),
            schema_version: 1,
            schema_checksum: "test-checksum".to_owned(),
            timestamp: "2026-01-02T00:00:00Z".parse().expect("timestamp"),
            mode: storage::BackupMode::Directory,
            tables: Vec::new(),
        };
        std::fs::write(
            backup.join("manifest.json"),
            serde_json::to_vec(&manifest).expect("manifest json"),
        )
        .expect("write manifest");
        let sources = SaturationSources::real(
            opened.state.feed_events.clone(),
            opened.state.media.clone(),
            media_root,
            Some(backup_root),
            opened.pool_observer,
        );
        let snapshot = RwLock::new(SaturationSnapshot::default());

        sample_saturation_once(&sources, &snapshot).await;

        let snapshot = read_snapshot(&snapshot);
        assert_eq!(snapshot.feed_queue_depth, Some(1));
        assert_eq!(
            snapshot.backup_last_success_timestamp,
            Some(manifest.timestamp.value().timestamp())
        );
        assert_eq!(snapshot.media_storage_bytes, Some(0));
        assert_eq!(snapshot.media_filesystem_bytes, Some(6));
        assert!(snapshot.db_pool_max.unwrap_or_default() > 0);
    }

    #[rstest::rstest]
    #[case::sqlite(storage::test_support::Backend::Sqlite)]
    #[tokio::test]
    async fn saturation_sampler_real_sources_clear_unconfigured_backup(
        #[case] backend: storage::test_support::Backend,
    ) {
        let base = tempfile::TempDir::new().expect("tempdir");
        let options = match backend {
            storage::test_support::Backend::Sqlite => storage::test_support::sqlite_url(&base),
            storage::test_support::Backend::Postgres => {
                unreachable!("sqlite_only supplies only SQLite")
            }
        };
        let opened = storage::open_database_with_observer(&options)
            .await
            .expect("open database");
        let media_root = base.path().join("media");
        std::fs::create_dir(&media_root).expect("media directory");
        let sources = SaturationSources::real(
            opened.state.feed_events.clone(),
            opened.state.media.clone(),
            media_root,
            None,
            opened.pool_observer,
        );
        let snapshot = seeded_snapshot();

        sample_saturation_once(&sources, &snapshot).await;

        let snapshot = read_snapshot(&snapshot);
        assert_eq!(snapshot.feed_queue_depth, Some(0));
        assert_eq!(snapshot.backup_last_success_timestamp, None);
        assert_eq!(snapshot.media_storage_bytes, Some(0));
        assert_eq!(snapshot.media_filesystem_bytes, Some(0));
        assert!(snapshot.db_pool_max.unwrap_or_default() > 0);
    }

    #[tokio::test]
    async fn saturation_sampler_spawn_samples_until_aborted() {
        let snapshot = Arc::new(RwLock::new(SaturationSnapshot::default()));
        let handle =
            spawn_saturation_sampler(SaturationSources::fake_success(sample()), snapshot.clone());

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if read_snapshot(&snapshot).feed_queue_depth == Some(7) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("sampler tick");

        handle.abort();
        let _ = handle.await;
    }
}

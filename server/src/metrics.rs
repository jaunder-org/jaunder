use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use host::metrics::SaturationSnapshot;
use storage::{DbPoolObserver, DbPoolSnapshot};
use tokio::task::JoinHandle;

use crate::backup::latest_successful_backup_timestamp;

const FEED_CLAIM_LEASE_TIMEOUT: chrono::Duration = chrono::Duration::minutes(5);
const SATURATION_SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

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
    state: Arc<storage::AppState>,
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
}

#[cfg(test)]
#[derive(Clone)]
struct FakeSaturationSources {
    sample: SaturationSample,
    backup_configured: bool,
    failure: Option<FakeSaturationFailure>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum FakeSaturationFailure {
    Feed,
    Backup,
    DbPool,
    Media,
}

impl SaturationSources {
    #[must_use]
    pub fn real(
        state: Arc<storage::AppState>,
        backup_destination_root: Option<PathBuf>,
        db_pool: DbPoolObserver,
    ) -> Self {
        Self {
            inner: SaturationSourcesInner::Real(RealSaturationSources {
                state,
                backup_destination_root,
                db_pool,
            }),
        }
    }

    async fn feed_queue_depth(&self) -> anyhow::Result<u64> {
        match &self.inner {
            SaturationSourcesInner::Real(real) => real
                .state
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
                    .map(|timestamp| timestamp.map(|timestamp| timestamp.timestamp()))
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
                .state
                .media
                .total_upload_bytes()
                .await
                .map(|total| u64::try_from(total.value()).unwrap_or(u64::MAX))
                .map_err(Into::into),
            #[cfg(test)]
            SaturationSourcesInner::Fake(fake) => fake.read_media_storage_bytes(),
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
}

pub async fn sample_saturation_once(
    sources: &SaturationSources,
    snapshot: &RwLock<SaturationSnapshot>,
) {
    let feed_queue_depth = sources.feed_queue_depth().await;
    let backup_last_success_timestamp = sources.backup_last_success_timestamp();
    let db_pool = sources.db_pool();
    let media_storage_bytes = sources.media_storage_bytes().await;

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
}

#[must_use]
pub fn spawn_saturation_sampler(
    sources: SaturationSources,
    snapshot: Arc<RwLock<SaturationSnapshot>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SATURATION_SAMPLE_INTERVAL);
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
    }

    #[tokio::test]
    async fn saturation_sampler_updates_snapshot_on_success() {
        let sources = SaturationSources::fake_success(sample());
        let snapshot = RwLock::new(SaturationSnapshot::default());

        sample_saturation_once(&sources, &snapshot).await;

        assert_success_snapshot(&read_snapshot(&snapshot));
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
        assert!(
            !trace_text(&output).contains("server.metrics.backup_last_success"),
            "unconfigured backup destination should not report a failure"
        );
    }

    #[tokio::test]
    async fn saturation_sampler_clears_only_failed_source() {
        let cases: [(&str, SaturationSources); 4] = [
            ("feed", SaturationSources::fake_feed_failure(sample())),
            ("backup", SaturationSources::fake_backup_failure(sample())),
            ("db_pool", SaturationSources::fake_db_pool_failure(sample())),
            ("media", SaturationSources::fake_media_failure(sample())),
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
        }
    }

    #[tokio::test]
    async fn saturation_sampler_reports_fixed_context_on_failure() {
        let cases: [(&str, SaturationSources); 4] = [
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
        let backup_root = base.path().join("backups");
        let backup = backup_root.join("backup-20260102T000000Z");
        std::fs::create_dir_all(&backup).expect("backup dir");
        let manifest = storage::BackupManifest {
            version: "0.1.0".to_owned(),
            schema_version: 1,
            schema_checksum: "test-checksum".to_owned(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
                .expect("timestamp")
                .to_utc(),
            mode: storage::BackupMode::Directory,
            tables: Vec::new(),
        };
        std::fs::write(
            backup.join("manifest.json"),
            serde_json::to_vec(&manifest).expect("manifest json"),
        )
        .expect("write manifest");
        let sources =
            SaturationSources::real(opened.state, Some(backup_root), opened.pool_observer);
        let snapshot = RwLock::new(SaturationSnapshot::default());

        sample_saturation_once(&sources, &snapshot).await;

        let snapshot = read_snapshot(&snapshot);
        assert_eq!(snapshot.feed_queue_depth, Some(1));
        assert_eq!(
            snapshot.backup_last_success_timestamp,
            Some(manifest.timestamp.timestamp())
        );
        assert_eq!(snapshot.media_storage_bytes, Some(0));
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
        let sources = SaturationSources::real(opened.state, None, opened.pool_observer);
        let snapshot = seeded_snapshot();

        sample_saturation_once(&sources, &snapshot).await;

        let snapshot = read_snapshot(&snapshot);
        assert_eq!(snapshot.feed_queue_depth, Some(0));
        assert_eq!(snapshot.backup_last_success_timestamp, None);
        assert_eq!(snapshot.media_storage_bytes, Some(0));
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

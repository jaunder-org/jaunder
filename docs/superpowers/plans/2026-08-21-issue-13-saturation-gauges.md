# Saturation Gauges Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add serve-only OpenTelemetry observable gauges for queue, backup,
database-pool, and DB-declared media storage saturation.

**Architecture:** Keep metric names and synchronous OTel observable callbacks in
`host::metrics`, but keep application dependency ownership in the server
composition root. Async DB/filesystem reads run in a serve-owned sampler task
that updates an in-memory snapshot; OTel callbacks only observe snapshot values.

**Tech Stack:** Rust, `opentelemetry` 0.30 observable gauges, `sqlx` pools,
Tokio background task, existing dual-backend storage harness, existing
`devtool run -- cargo xtask check` gate.

**Scope:** In: #13 saturation gauges, DB-declared media bytes, backup manifest
timestamp gauge, docs. Out: on-disk media usage collection, now tracked as
#1103.

**Task summary:** Task 1 adds feed-event queue-depth storage reads. Task 2 adds
DB-declared media byte reads. Task 3 adds backup artifact timestamp discovery.
Task 4 exposes a storage-owned opened-database handle with a backend-erased pool
observer for serve. Task 5 adds the metrics snapshot, observable registration,
and sampler. Task 6 wires serve and documentation.

**Key risks/decisions:** OTel callbacks are synchronous, so no async work may
happen inside `with_callback`. `AppState` must not grow a raw pool field. Gauge
read failures clear a snapshot value and emit no datapoint. Backup timestamps
come from `manifest.json`, not filesystem mtime or name sorting.

## Global Constraints

- Preserve backend parity: storage read methods must be covered for SQLite and
  PostgreSQL through the shared storage harness.
- Do not add storage traits, `AppState`, or higher workspace abstractions to
  `host`.
- Do not add the live pool or pool observer to `AppState` or request handler
  dependencies.
- Do not put async DB or filesystem reads inside OTel observable callbacks.
- On sample failure, emit no datapoint for that gauge and report one fixed
  diagnostic context with no path, URL, user input, or caller-supplied text.
- Use `devtool run -- <cmd>` for test and gate commands.
- Before each commit: tick completed plan checkboxes, run
  `devtool run -- cargo xtask check`, inspect/stage formatter changes, then
  commit without a `Co-Authored-By` trailer.

---

### Task 1: Feed-Event Queue-Depth Storage Read

**Files:**

- Modify: `storage/src/feed_events.rs`
- Modify: `storage/src/sqlite/feed_events.rs`
- Modify: `storage/src/postgres/feed_events.rs`
- Test: `storage/src/feed_events.rs`

**Interfaces:**

- Produces:
  `async fn claimable_count(&self, lease_timeout: chrono::Duration) -> Result<u64, FeedEventError>`
  on `FeedEventStorage`.
- Produces:
  `async fn claimable_count(pool: &sqlx::Pool<Self>, now: chrono::DateTime<chrono::Utc>, lease_cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64, FeedEventError>`
  on `FeedEventDialect`.
- Consumes existing claimability rule from
  `FeedEventStorage::claim_pending_batch`.

- [x] **Step 1: Write failing dual-backend tests**

  In `storage/src/feed_events.rs`, add tests using `#[apply(backends)]`:

  ```rust
  #[apply(backends)]
  async fn claimable_count_counts_pending_ready_rows(#[case] backend: Backend) {
      let env = backend.setup().await;
      env.state.feed_events.enqueue(&fp("/feed.rss")).await.unwrap();

      let count = env
          .state
          .feed_events
          .claimable_count(chrono::Duration::minutes(5))
          .await
          .unwrap();

      assert_eq!(count, 1);
  }

  #[apply(backends)]
  async fn claimable_count_ignores_delayed_retries(#[case] backend: Backend) {
      let env = backend.setup().await;
      let id = env.state.feed_events.enqueue(&fp("/feed.rss")).await.unwrap();
      env.state
          .feed_events
          .mark_failed(&[id], "retry later", chrono::Utc::now() + chrono::Duration::hours(1))
          .await
          .unwrap();

      let count = env
          .state
          .feed_events
          .claimable_count(chrono::Duration::minutes(5))
          .await
          .unwrap();

      assert_eq!(count, 0);
  }

  #[apply(backends)]
  async fn claimable_count_ignores_live_claims(#[case] backend: Backend) {
      let env = backend.setup().await;
      env.state.feed_events.enqueue(&fp("/feed.rss")).await.unwrap();
      let claimed = env
          .state
          .feed_events
          .claim_pending_batch(10, chrono::Duration::minutes(5))
          .await
          .unwrap();
      assert_eq!(claimed.len(), 1);

      let count = env
          .state
          .feed_events
          .claimable_count(chrono::Duration::minutes(5))
          .await
          .unwrap();

      assert_eq!(count, 0);
  }

  #[apply(backends)]
  async fn claimable_count_counts_expired_claims(#[case] backend: Backend) {
      let env = backend.setup().await;
      env.state.feed_events.enqueue(&fp("/feed.rss")).await.unwrap();
      env.state
          .feed_events
          .claim_pending_batch(10, chrono::Duration::minutes(5))
          .await
          .unwrap();

      let count = env
          .state
          .feed_events
          .claimable_count(chrono::Duration::zero())
          .await
          .unwrap();

      assert_eq!(count, 1);
  }
  ```

- [x] **Step 2: Run the focused tests and verify failure**

  Run: `devtool run -- cargo nextest run -p storage claimable_count`

  Expected: FAIL because `claimable_count` is not defined.

- [x] **Step 3: Implement feed-event queue-depth reads**

  Add the trait methods above. Implement per dialect with a read-only scalar
  query matching the `claim_pending_batch` predicate exactly:

  ```sql
  SELECT COUNT(*)
  FROM feed_events
  WHERE (status = 'pending' AND next_attempt_at <= $1)
     OR (status = 'claimed' AND claimed_at < $2)
  ```

  For SQLite and PostgreSQL, bind `now` and `lease_cutoff` using the same order
  and timestamp types already used by the claim queries. Convert the returned
  count to `u64` with saturation or checked conversion that cannot panic on
  unexpected negative DB results.

- [x] **Step 4: Run the focused tests and verify pass**

  Run: `devtool run -- cargo nextest run -p storage claimable_count`

  Expected: PASS.

- [x] **Step 5: Gate and commit**

  Run: `devtool run -- cargo xtask check`

  Stage the plan checkbox update and storage changes, then commit:
  `git add docs/superpowers/plans/2026-08-21-issue-13-saturation-gauges.md storage/src/feed_events.rs storage/src/sqlite/feed_events.rs storage/src/postgres/feed_events.rs`

  Commit message: `feat(observability): expose feed queue depth`

### Task 2: DB-Declared Media Storage Bytes

**Files:**

- Modify: `storage/src/media.rs`
- Modify: `storage/src/sqlite/media.rs`
- Modify: `storage/src/postgres/media.rs`
- Test: `storage/src/media.rs`

**Interfaces:**

- Produces:
  `async fn total_upload_bytes(&self) -> sqlx::Result<common::media::ByteSize>`
  on `MediaStorage`.
- Produces:
  `async fn total_upload_bytes(pool: &sqlx::Pool<Self>) -> sqlx::Result<common::media::ByteSize>`
  on `MediaDialect`.
- Consumes existing `media.size_bytes` and `source = 'upload'` schema.

- [x] **Step 1: Write failing dual-backend tests**

  Add tests in `storage/src/media.rs`:

  ```rust
  #[apply(backends)]
  async fn total_upload_bytes_sums_upload_rows(#[case] backend: Backend) {
      let env = backend.setup().await;
      let [alice] = seed_users(&env.state).await;
      seed_media(&env.state, alice, "a.jpg").await;
      seed_media(&env.state, alice, "b.jpg").await;

      let total = env.state.media.total_upload_bytes().await.unwrap();

      assert_eq!(total, parse_byte_size("2"));
  }

  #[apply(backends)]
  async fn total_upload_bytes_excludes_non_upload_sources(#[case] backend: Backend) {
      let env = backend.setup().await;
      let [alice] = seed_users(&env.state).await;
      seed_media(&env.state, alice, "upload.jpg").await;
      env.base
          .execute(
              "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
               VALUES (1, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', \
                       'remote.jpg', 'cached', 'image/jpeg', 99)",
          )
          .await
          .unwrap();

      let total = env.state.media.total_upload_bytes().await.unwrap();

      assert_eq!(total, parse_byte_size("1"));
  }
  ```

- [x] **Step 2: Run the focused tests and verify failure**

  Run: `devtool run -- cargo nextest run -p storage total_upload_bytes`

  Expected: FAIL because `total_upload_bytes` is not defined.

- [x] **Step 3: Implement DB-declared media byte reads**

  Add the trait methods above. Reuse the same backend divergence as
  `get_user_upload_usage`: SQLite can use `COALESCE(SUM(size_bytes), 0)`;
  PostgreSQL needs the explicit `::bigint` cast. Query all users, but only rows
  whose `source = 'upload'`. Decode into `ByteSize`, so negative DB tampering
  fails rather than being silently exported.

- [x] **Step 4: Run the focused tests and verify pass**

  Run: `devtool run -- cargo nextest run -p storage total_upload_bytes`

  Expected: PASS.

- [x] **Step 5: Gate and commit**

  Run: `devtool run -- cargo xtask check`

  Stage the plan checkbox update and media changes, then commit:
  `git add docs/superpowers/plans/2026-08-21-issue-13-saturation-gauges.md storage/src/media.rs storage/src/sqlite/media.rs storage/src/postgres/media.rs`

  Commit message: `feat(observability): expose media storage bytes`

### Task 3: Backup Last-Success Timestamp Discovery

**Files:**

- Modify: `server/src/backup.rs`
- Test: `server/src/backup.rs`

**Interfaces:**

- Produces:
  `fn latest_successful_backup_timestamp(destination_root: &std::path::Path) -> anyhow::Result<Option<chrono::DateTime<chrono::Utc>>>`
  or a narrower local error type if the implementation keeps it private.
- Consumes `storage::backup::BackupManifest` JSON shape with `timestamp`.
- Consumes existing backup artifact conventions from `backup_path_for_mode` and
  `prune_backups`.

- [x] **Step 1: Write failing backup artifact tests**

  Add tests in `server/src/backup.rs`:

  ```rust
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

      assert_eq!(timestamp.to_rfc3339(), "2026-01-02T00:00:00+00:00");
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

      assert_eq!(timestamp.to_rfc3339(), "2026-01-02T00:00:00+00:00");
  }

  #[test]
  fn latest_successful_backup_timestamp_ignores_suffix_only_archive() {
      let temp = TempDir::new().expect("tempdir");
      std::fs::write(temp.path().join("backup-20260102T000000Z.tar.gz"), b"not a tar")
          .expect("write bad archive");

      let timestamp = latest_successful_backup_timestamp(temp.path()).expect("timestamp scan");

      assert_eq!(timestamp, None);
  }

  #[test]
  fn latest_successful_backup_timestamp_ignores_malformed_manifest() {
      let temp = TempDir::new().expect("tempdir");
      let bad = temp.path().join("backup-20260102T000000Z");
      std::fs::create_dir(&bad).expect("dir");
      std::fs::write(bad.join("manifest.json"), b"{").expect("bad manifest");

      let timestamp = latest_successful_backup_timestamp(temp.path()).expect("timestamp scan");

      assert_eq!(timestamp, None);
  }
  ```

  Add private test helpers `write_test_manifest` and `write_test_archive` that
  write a minimal valid `BackupManifest` JSON. Do not use filesystem mtime in
  assertions.

- [x] **Step 2: Run the focused tests and verify failure**

  Run:
  `devtool run -- cargo nextest run -p jaunder latest_successful_backup_timestamp`

  Expected: FAIL because the helper does not exist.

- [x] **Step 3: Implement manifest-based artifact scanning**

  Implement `latest_successful_backup_timestamp` in `server/src/backup.rs`. Scan
  children of `destination_root`; return `Ok(None)` for a missing root. For
  directories, read `manifest.json`. For `.tar.gz`, open the gzip tarball and
  read the `manifest.json` member. Parse `BackupManifest` and keep the max
  `manifest.timestamp`.

  Malformed individual artifacts are not successful backups for this gauge: skip
  them and report a fixed `server.metrics.backup_last_success` diagnostic once
  per scan if at least one artifact failed to read or parse. A failed
  `read_dir(destination_root)` is a scan failure and returns `Err`.

- [x] **Step 4: Run the focused tests and verify pass**

  Run:
  `devtool run -- cargo nextest run -p jaunder latest_successful_backup_timestamp`

  Expected: PASS.

- [x] **Step 5: Gate and commit**

  Run: `devtool run -- cargo xtask check`

  Stage the plan checkbox update and backup changes, then commit:
  `git add docs/superpowers/plans/2026-08-21-issue-13-saturation-gauges.md server/src/backup.rs`

  Commit message: `feat(observability): read backup success timestamp`

### Task 4: Storage-Owned Database Handle And Pool Observer

**Files:**

- Modify: `storage/src/db.rs`
- Modify: `server/src/commands.rs`
- Test: `storage/src/db.rs`
- Test: `server/src/commands.rs`

**Interfaces:**

- Produces in `storage/src/db.rs`:

  ```rust
  #[derive(Clone)]
  pub struct DbPoolObserver {
      inner: DbPoolObserverInner,
  }

  enum DbPoolObserverInner {
      Sqlite(sqlx::SqlitePool),
      Postgres(sqlx::PgPool),
  }

  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
  pub struct DbPoolSnapshot {
      pub used: u64,
      pub idle: u64,
      pub max: u64,
  }

  impl DbPoolObserver {
      pub fn snapshot(&self) -> DbPoolSnapshot;
  }

  pub struct OpenedDatabase {
      pub state: std::sync::Arc<AppState>,
      pub pool_observer: DbPoolObserver,
  }

  pub async fn open_database_with_observer(
      opts: &DbConnectOptions,
  ) -> sqlx::Result<OpenedDatabase>;

  pub async fn open_existing_database_with_observer(
      opts: &DbConnectOptions,
  ) -> sqlx::Result<OpenedDatabase>;
  ```

- Consumes `sqlx::Pool::size()`, `sqlx::Pool::num_idle()`, and
  `sqlx::Pool::options().get_max_connections()`.
- Later tasks consume `OpenedDatabase.state` for existing wiring and
  `OpenedDatabase.pool_observer` for metrics sampling.

- [x] **Step 1: Write failing storage observer tests**

  Add tests in `storage/src/db.rs`:

  ```rust
  #[apply(sqlite_only)]
  async fn opened_sqlite_database_carries_pool_observer(#[case] backend: Backend) {
      let env = backend.setup().await;
      let opened = open_existing_database_with_observer(&env.base.db_options())
          .await
          .expect("open existing database with observer");

      let snapshot = opened.pool_observer.snapshot();

      assert!(snapshot.max >= 1);
      assert!(snapshot.used <= snapshot.max);
      assert!(snapshot.idle <= snapshot.max);
      assert!(std::sync::Arc::strong_count(&opened.state) >= 1);
  }

  #[apply(postgres_only)]
  async fn opened_postgres_database_carries_pool_observer(#[case] backend: Backend) {
      let env = backend.setup().await;
      let opened = open_existing_database_with_observer(&env.base.db_options())
          .await
          .expect("open existing database with observer");

      let snapshot = opened.pool_observer.snapshot();

      assert!(snapshot.max >= 1);
      assert!(snapshot.used <= snapshot.max);
      assert!(snapshot.idle <= snapshot.max);
      assert!(std::sync::Arc::strong_count(&opened.state) >= 1);
  }
  ```

  If `DbConnectOptions` is not directly available from the test base, add a
  small test-support accessor rather than reconstructing URLs from strings.

- [x] **Step 2: Run the focused storage tests and verify failure**

  Run: `devtool run -- cargo nextest run -p storage pool_observer`

  Expected: FAIL because `OpenedDatabase` and `DbPoolObserver` are not defined.

- [x] **Step 3: Implement storage-owned opened database handle**

  In `storage/src/db.rs`, implement the public `open_*_with_observer` functions
  by calling the existing crate-private `sqlite::open_sqlite_database_with_pool`
  and `postgres::open_postgres_database_with_pool` functions. Keep the existing
  `open_database` and `open_existing_database` APIs returning only
  `Arc<AppState>` for existing callers.

  In `server/src/commands.rs`, change the real `StartupDatabaseOperations` path
  to return `storage::OpenedDatabase`. Keep test doubles simple by adding a
  server-local wrapper if needed:

  ```rust
  struct StartupDatabase {
      state: Arc<storage::AppState>,
      pool_observer: Option<storage::DbPoolObserver>,
  }
  ```

  Real serve opens must produce `Some(pool_observer)` from
  `storage::OpenedDatabase`. Test-only fake database operations used by existing
  startup-error tests may produce `None` if they never reach metrics
  registration. Do not add the observer to `AppState`, Axum extensions, or
  Leptos contexts.

- [x] **Step 4: Run focused storage and command tests**

  Run: `devtool run -- cargo nextest run -p storage pool_observer`

  Run:
  `devtool run -- cargo nextest run -p jaunder open_server_database prepare_server`

  Expected: PASS.

- [x] **Step 5: Gate and commit**

  Run: `devtool run -- cargo xtask check`

  Stage the plan checkbox update, storage observer, and command changes, then
  commit:
  `git add docs/superpowers/plans/2026-08-21-issue-13-saturation-gauges.md storage/src/db.rs server/src/commands.rs`

  Commit message: `feat(observability): retain serve pool observer`

### Task 5: Metrics Snapshot, Observable Instruments, And Sampler

**Files:**

- Modify: `host/src/metrics.rs`
- Modify: `server/src/observability.rs`
- Create: `server/src/metrics.rs`
- Modify: `server/src/lib.rs`
- Test: `host/src/metrics.rs`
- Test: `server/src/metrics.rs`

**Interfaces:**

- Produces in `host::metrics`:

  ```rust
  #[derive(Clone, Debug, Default)]
  pub struct SaturationSnapshot {
      pub feed_queue_depth: Option<u64>,
      pub backup_last_success_timestamp: Option<i64>,
      pub db_pool_used: Option<u64>,
      pub db_pool_idle: Option<u64>,
      pub db_pool_max: Option<u64>,
      pub media_storage_bytes: Option<u64>,
  }

  pub fn register_saturation_observables(
      snapshot: std::sync::Arc<std::sync::RwLock<SaturationSnapshot>>,
  ) -> SaturationObservableGuard;
  ```

- Produces a guard type that owns the observable instruments so callbacks remain
  registered for the serve lifetime.
- Produces a server sampler task that updates the shared snapshot from
  `FeedEventStorage`, `MediaStorage`, backup discovery, and `DbPoolObserver`.
- Produces private server test helpers:

  ```rust
  #[derive(Clone, Copy, Default)]
  struct SaturationSample {
      feed_queue_depth: u64,
      backup_last_success_timestamp: i64,
      db_pool: DbPoolSnapshot,
      media_storage_bytes: u64,
  }

  impl SaturationSources {
      fn fake_success(sample: SaturationSample) -> Self;
      fn fake_backup_unconfigured(sample: SaturationSample) -> Self;
      fn fake_feed_failure(sample: SaturationSample) -> Self;
      fn fake_backup_failure(sample: SaturationSample) -> Self;
      fn fake_db_pool_failure(sample: SaturationSample) -> Self;
      fn fake_media_failure(sample: SaturationSample) -> Self;
  }
  ```

- [ ] **Step 1: Write failing host metric export test**

  Extend the single `host/src/metrics.rs` metrics test rather than adding a
  second process-global provider install. Add assertions that after
  `register_saturation_observables(snapshot)` and `provider.force_flush()`, the
  exported instrument names include:

  ```rust
  "jaunder.feed.queue_depth"
  "jaunder.backup.last_success_timestamp"
  "jaunder.db.pool.used"
  "jaunder.db.pool.idle"
  "jaunder.db.pool.max"
  "jaunder.media.storage_bytes"
  ```

  Add a branch where one snapshot field is `None` and assert that instrument has
  no datapoint, not a zero datapoint.

- [ ] **Step 2: Run host metrics test and verify failure**

  Run:
  `devtool run -- cargo nextest run -p host every_emitter_exports_its_instrument`

  Expected: FAIL because observable registration is not implemented.

- [ ] **Step 3: Implement `host::metrics` observable registration**

  Build the six observable gauges from `global::meter("jaunder")`. Use `u64`
  gauges for queue depth, DB pool counts, and media bytes. Use an `i64` gauge
  with unit `s` for `jaunder.backup.last_success_timestamp`. In each
  `with_callback`, take a read lock, observe only `Some` values, and return
  immediately. Hold every instrument in `SaturationObservableGuard`.

- [ ] **Step 4: Write failing server sampler tests**

  Add server tests around pure sampler update logic:

  ```rust
  #[tokio::test]
  async fn saturation_sampler_updates_snapshot_on_success() {
      let snapshot = Arc::new(RwLock::new(host::metrics::SaturationSnapshot::default()));
      let sources = SaturationSources::fake_success(SaturationSample {
          feed_queue_depth: 7,
          backup_last_success_timestamp: 1_767_225_600,
          db_pool: DbPoolSnapshot {
              used: 2,
              idle: 3,
              max: 10,
          },
          media_storage_bytes: 42,
      });

      sample_saturation_once(&sources, &snapshot).await;

      let observed = snapshot.read().expect("snapshot");
      assert_eq!(observed.feed_queue_depth, Some(7));
      assert_eq!(observed.backup_last_success_timestamp, Some(1_767_225_600));
      assert_eq!(observed.db_pool_used, Some(2));
      assert_eq!(observed.db_pool_idle, Some(3));
      assert_eq!(observed.db_pool_max, Some(10));
      assert_eq!(observed.media_storage_bytes, Some(42));
  }

  #[tokio::test]
  async fn saturation_sampler_clears_backup_timestamp_when_unconfigured() {
      let snapshot = Arc::new(RwLock::new(host::metrics::SaturationSnapshot {
          feed_queue_depth: Some(1),
          backup_last_success_timestamp: Some(2),
          db_pool_used: Some(3),
          db_pool_idle: Some(4),
          db_pool_max: Some(5),
          media_storage_bytes: Some(6),
      }));
      let sources = SaturationSources::fake_backup_unconfigured(SaturationSample {
          feed_queue_depth: 7,
          db_pool: DbPoolSnapshot {
              used: 2,
              idle: 3,
              max: 10,
          },
          media_storage_bytes: 42,
      });

      sample_saturation_once(&sources, &snapshot).await;

      let observed = snapshot.read().expect("snapshot");
      assert_eq!(observed.feed_queue_depth, Some(7));
      assert_eq!(observed.backup_last_success_timestamp, None);
      assert_eq!(observed.db_pool_used, Some(2));
      assert_eq!(observed.db_pool_idle, Some(3));
      assert_eq!(observed.db_pool_max, Some(10));
      assert_eq!(observed.media_storage_bytes, Some(42));
  }

  #[tokio::test]
  async fn saturation_sampler_clears_only_failed_source() {
      let snapshot = Arc::new(RwLock::new(host::metrics::SaturationSnapshot::default()));
      let sources = SaturationSources::fake_media_failure(SaturationSample::default());

      sample_saturation_once(&sources, &snapshot).await;

      let observed = snapshot.read().expect("snapshot");
      assert_eq!(observed.feed_queue_depth, Some(0));
      assert_eq!(observed.backup_last_success_timestamp, Some(0));
      assert_eq!(observed.db_pool_used, Some(0));
      assert_eq!(observed.db_pool_idle, Some(0));
      assert_eq!(observed.db_pool_max, Some(0));
      assert_eq!(observed.media_storage_bytes, None);
  }

  #[tokio::test]
  async fn saturation_sampler_reports_fixed_context_on_failure() {
      let snapshot = Arc::new(RwLock::new(host::metrics::SaturationSnapshot::default()));
      let sources = SaturationSources::fake_media_failure(SaturationSample::default());

      let (_result, trace) = crate::helpers::swallowed_test::capture_async(
          sample_saturation_once(&sources, &snapshot),
      )
      .await;

      assert!(trace.contains(r#""error.context":"server.metrics.media_storage_bytes""#));
  }
  ```

  The first test should prove all six fields are populated from successful
  source reads. The unconfigured-backup test should model
  `backup.destination_path = None`, clear only `backup_last_success_timestamp`,
  and emit no failure diagnostic because an unconfigured destination is normal.
  The failed-source test should seed source failure cases for feed, backup, DB
  pool, and media and assert only the failed gauge field is cleared while
  successful fields remain current.

  Add a table-driven diagnostic assertion for all fixed contexts:

  | Fake source constructor                   | Cleared field(s)                | Expected context                     |
  | ----------------------------------------- | ------------------------------- | ------------------------------------ |
  | `SaturationSources::fake_feed_failure`    | `feed_queue_depth`              | `server.metrics.feed_queue_depth`    |
  | `SaturationSources::fake_backup_failure`  | `backup_last_success_timestamp` | `server.metrics.backup_last_success` |
  | `SaturationSources::fake_db_pool_failure` | DB pool used/idle/max           | `server.metrics.db_pool`             |
  | `SaturationSources::fake_media_failure`   | `media_storage_bytes`           | `server.metrics.media_storage_bytes` |

- [ ] **Step 5: Implement the serve-owned sampler**

  Implement a sampler function that performs one sample tick and a spawn helper:

  ```rust
  async fn sample_saturation_once(sources: &SaturationSources, snapshot: &RwLock<SaturationSnapshot>);

  fn spawn_saturation_sampler(
      sources: SaturationSources,
      snapshot: Arc<RwLock<SaturationSnapshot>>,
  ) -> tokio::task::JoinHandle<()>;
  ```

  `SaturationSources` owns cloned storage handles, an optional backup
  destination root resolver, and `DbPoolObserver`. Map `None` from the backup
  destination resolver to `backup_last_success_timestamp = None` without calling
  `latest_successful_backup_timestamp` and without reporting an error. Wrap the
  infallible real `DbPoolObserver::snapshot()` in a sampler source method that
  returns `Result<DbPoolSnapshot, SaturationError>` so the common failure path
  and fixed diagnostic context are testable. Use one conservative interval
  constant in `server/src/metrics.rs`:

  ```rust
  const SATURATION_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
  ```

  Do not spawn the sampler when no OTLP endpoint is configured.

- [ ] **Step 6: Run focused metrics tests**

  Run:
  `devtool run -- cargo nextest run -p host every_emitter_exports_its_instrument`

  Run: `devtool run -- cargo nextest run -p jaunder saturation_sampler`

  Expected: PASS.

- [ ] **Step 7: Gate and commit**

  Run: `devtool run -- cargo xtask check`

  Stage the plan checkbox update and metrics/sampler changes, then commit:
  `git add docs/superpowers/plans/2026-08-21-issue-13-saturation-gauges.md host/src/metrics.rs server/src/observability.rs server/src/lib.rs server/src/metrics.rs`

  Commit message: `feat(observability): register saturation gauges`

### Task 6: Serve Wiring And Documentation

**Files:**

- Modify: `server/src/commands.rs`
- Modify: `docs/observability.md`
- Modify: `docs/ARCHITECTURE.md`
- Test: `server/src/commands.rs`

**Interfaces:**

- Consumes `ServerDatabase { state, pool_observer }` from Task 4.
- Consumes `host::metrics::register_saturation_observables` and server sampler
  from Task 5.
- Produces serve lifetime ownership of `SaturationObservableGuard` and sampler
  `JoinHandle`, probably in `PreparedServer`.
- Produces:

  ```rust
  struct PreparedSaturationMetrics {
      _observables: host::metrics::SaturationObservableGuard,
      _sampler: tokio::task::JoinHandle<()>,
  }

  pub struct PreparedServer {
      pub saturation_metrics: Option<PreparedSaturationMetrics>,
      // existing fields unchanged
  }
  ```

- Produces or reuses a private test helper
  `fn test_storage_args() -> StorageArgs` that prepares isolated SQLite-backed
  storage arguments for `prepare_server` tests.

- [ ] **Step 1: Write failing serve wiring tests**

  Add or extend `prepare_server` tests to prove:

  ```rust
  #[tokio::test]
  async fn prepare_server_registers_saturation_sampler_when_otel_endpoint_is_set() {
      common::test_support::with_env(|env| async move {
          env.set("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318");
          let storage = test_storage_args();
          let prepared = prepare_server(
              &storage,
              "127.0.0.1:0".parse().expect("bind"),
              false,
              None,
          )
          .await
          .expect("prepare server");

          assert!(prepared.saturation_metrics.is_some());
      })
      .await;
  }

  #[tokio::test]
  async fn prepare_server_does_not_start_saturation_sampler_without_otel_endpoint() {
      common::test_support::with_env(|env| async move {
          env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
          env.remove("OTEL_EXPORTER_OTLP_ENDPOINT");
          let storage = test_storage_args();
          let prepared = prepare_server(
              &storage,
              "127.0.0.1:0".parse().expect("bind"),
              false,
              None,
          )
          .await
          .expect("prepare server");

          assert!(prepared.saturation_metrics.is_none());
      })
      .await;
  }
  ```

  Use `common::test_support::with_env` to set/remove
  `JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT`. Do not require a live collector; assert
  owned guard/sampler presence through `PreparedServer` state or a small
  test-visible constructor boundary.

- [ ] **Step 2: Run serve wiring tests and verify failure**

  Run:
  `devtool run -- cargo nextest run -p jaunder saturation_sampler prepare_server`

  Expected: FAIL because serve wiring is not complete.

- [ ] **Step 3: Wire serve setup**

  In `prepare_server`, after opening the database and before returning
  `PreparedServer`, register saturation observables and spawn the sampler only
  when the same OTLP endpoint gate used by telemetry setup is configured. Hold
  the observable guard and sampler handle for the server lifetime in
  `PreparedServer`. Use `ServerDatabase.state` for existing router/worker wiring
  and `ServerDatabase.pool_observer` only for saturation sampling.

- [ ] **Step 4: Update docs**

  In `docs/observability.md`, document the six new gauge names, snapshot
  sampling, no-datapoint-on-failure behavior, and the DB-declared media bytes
  vs. #1103 on-disk usage split.

  In `docs/ARCHITECTURE.md` Observability/Metrics section, update the
  materialized architecture view with the serve-only sampler, `host::metrics`
  observable registration, pool observer outside `AppState`, and
  ADR-0011/ADR-0058 consistency.

- [ ] **Step 5: Run focused tests and docs format**

  Run:
  `devtool run -- cargo nextest run -p jaunder saturation_sampler prepare_server`

  Run: `devtool run -- prettier -w docs/observability.md docs/ARCHITECTURE.md`

  Expected: PASS for tests; Prettier exits 0.

- [ ] **Step 6: Full local gate**

  Run: `devtool run -- cargo xtask check`

  Expected: PASS.

- [ ] **Step 7: Commit**

  Stage the plan checkbox update, serve wiring, and docs:
  `git add docs/superpowers/plans/2026-08-21-issue-13-saturation-gauges.md server/src/commands.rs docs/observability.md docs/ARCHITECTURE.md`

  Commit message: `feat(observability): wire saturation sampler`

## Self-Review

- Spec coverage: Tasks 1 and 2 cover storage read APIs and dual-backend tests;
  Task 3 covers backup timestamp semantics and invalid artifacts; Task 4 covers
  the pool observer without widening `AppState`; Task 5 covers synchronous OTel
  callbacks and no-datapoint snapshot behavior; Task 6 covers serve-only wiring
  and documentation.
- Separable concerns: on-disk media usage was filed as #1103, triaged P3, and
  linked as blocked by #13.
- Placeholder scan: no task uses TBD/TODO-style placeholders; each task names
  concrete files, interfaces, test cases, commands, and expected results.
- Type consistency: later tasks consume `DbPoolObserver`, `DbPoolSnapshot`,
  `SaturationSnapshot`, and `register_saturation_observables` as produced by
  earlier tasks.

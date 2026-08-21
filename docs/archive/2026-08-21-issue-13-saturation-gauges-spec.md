# Issue #13: Saturation gauges via OTel async observables

## Context

Issue #13 is the deferred saturation-gauge slice from ADR-0011's metrics
pipeline. The existing `host::metrics` facade owns bounded, cardinality-safe
event metric instruments. `server::observability` installs the OTLP
`MeterProvider`, and `server::commands::prepare_server` is the composition root
that has the storage handles, database pool, and storage path needed to observe
live saturation state.

The new gauges must use OpenTelemetry observable instruments. In the pinned
OpenTelemetry Rust API, observable callbacks are synchronous; any async storage
or filesystem work must happen outside the callback. They are serve-process
operational signals, not one-shot CLI emits.

## Decisions

- Instrument names remain catalogued by `host::metrics`, keeping metrics
  vocabulary in one facade.
- Observable callbacks are registered from the server composition root, where
  the concrete application dependencies are already available.
- `host` must not learn storage traits, `AppState`, or other higher workspace
  abstractions. Any `host::metrics` API for observables accepts host-safe
  callback values only.
- Async gauge sources are sampled by a serve-owned background task into a small
  in-memory snapshot. The OTel observable callback synchronously observes the
  latest successful snapshot values and never blocks on async work.
- Sample read failures clear that source's snapshot value, so the next
  collection emits no datapoint for that gauge. They report one bounded
  diagnostic with a fixed context string and no user/path/input attributes.
- Saturation gauges are registered only for `serve`; one-shot CLI commands do
  not register live-state observables.
- Media byte accounting for this issue is DB-declared upload bytes. On-disk
  media usage is follow-up #1103.

## Required Gauges

### Feed-event queue depth

Expose `jaunder.feed.queue_depth` as the count of work claimable at collection
time:

- `status = 'pending'` and `next_attempt_at <= now`, plus
- `status = 'claimed'` and `claimed_at < now - lease_timeout`.

The query must use the same eligibility semantics as
`FeedEventStorage::claim_pending_batch`, without claiming or mutating rows.
Scheduled retries that are not yet claimable are not included.

### Backup last success timestamp

Expose `jaunder.backup.last_success_timestamp` as the Unix timestamp in seconds
from the manifest timestamp of the newest valid backup artifact under the
configured backup destination.

The value is derived from backup artifacts on disk, not persisted in
`site_config`. A valid directory backup has the marker shape already used by
backup pruning: a backup directory carrying `manifest.json`. The timestamp is
read from that manifest, not the directory name, filesystem mtime, or pruning
sort order.

Archive backups are valid only when they are readable `.tar.gz` backup archives
containing a `manifest.json`; the timestamp is read from that manifest. A file
that merely has a `.tar.gz` suffix is not enough for this gauge.

If no backup destination is configured, scheduled backups are disabled and the
gauge emits no datapoint.

### Database pool utilization

Expose database pool saturation as separate gauges:

- `jaunder.db.pool.used`
- `jaunder.db.pool.idle`
- `jaunder.db.pool.max`

The gauges are collected from the live `sqlx::Pool` owned by the serve process.
The implementation must preserve backend parity for SQLite and PostgreSQL.

### Media storage bytes

Expose `jaunder.media.storage_bytes` as the DB-declared total upload byte count:

- sum `media.size_bytes`
- include only upload media rows
- do not recursively walk `storage_path/media`

The value matches quota/accounting semantics. Filesystem usage and drift
diagnostics are owned by #1103.

## Storage Surface

Add explicit read methods only for storage facts:

- `FeedEventStorage::claimable_count(lease_timeout)` or an equivalent
  backend-parametric API that returns the queue-depth gauge value.
- `MediaStorage::total_upload_bytes()` or an equivalent backend-parametric API
  that returns the media byte gauge value.

These methods must be read-only, backend-parametric, and tested against both
SQLite and PostgreSQL through the shared storage harness.

Database-pool gauges should not be routed through storage traits. They observe
the live pool itself at the composition root.

To make that possible without widening `AppState`, the server database-open path
must return a serve-local database handle that contains both the `Arc<AppState>`
and a backend-erased pool observer for the live pool. The pool observer may know
raw `sqlx` pool types, but it must not become a general dependency bundle and
must not be layered into request handlers.

Backup artifact discovery may live in `server::backup` because it is a
server-side filesystem concern and already owns backup scheduling, pruning, and
artifact naming.

## Failure Reporting

Each gauge source has one fixed diagnostic context:

- `server.metrics.feed_queue_depth`
- `server.metrics.backup_last_success`
- `server.metrics.db_pool`
- `server.metrics.media_storage_bytes`

On sample failure, the sampler reports the failure through the existing bounded
swallowed-error path and clears that gauge's snapshot value. The next OTel
callback collection records no point for that gauge. It must not emit zero, a
sentinel, a stale value, a path, a URL, or caller-supplied text.

## Documentation

Update the observability documentation and architecture view to describe:

- the async observable registration path,
- the four saturation signals and exact metric names,
- no-datapoint-on-read-failure semantics,
- the distinction between DB-declared media upload bytes and on-disk media
  usage, citing #1103 for the latter.

No new ADR is required unless implementation uncovers a broader architectural
choice beyond ADR-0011's deferred saturation-gauge decision.

## Acceptance Criteria

- Starting `serve` with an OTLP metrics endpoint registers async observable
  instruments for feed-event queue depth, backup last-success timestamp,
  database pool used/idle/max, and DB-declared media storage bytes.
- The same binary with no OTLP endpoint keeps metrics inert and does not require
  the gauge dependencies to run collection work.
- Async storage and filesystem reads run in a serve-owned sampler task; OTel
  observable callbacks are synchronous and observe only the sampler's latest
  successful snapshot.
- Gauge callbacks are registered from the server composition root and do not
  move storage traits or `AppState` into `host`.
- Serve database setup exposes the live pool to metrics through a backend-erased
  pool observer without adding the pool or observer to `AppState`.
- Feed-event queue depth uses the same claimability rule as
  `claim_pending_batch` and has dual-backend tests for pending, delayed retry,
  claimed-with-live-lease, and expired-claim rows.
- Media storage bytes are computed from DB rows and have dual-backend tests that
  prove only upload media rows contribute.
- Backup last-success timestamp is derived from `manifest.json` timestamps in
  valid backup artifacts, emits no datapoint when backups are unconfigured or no
  valid artifact exists, and tests directory backups, readable archive backups,
  suffix-only invalid archives, and malformed manifests.
- Database pool gauges expose used, idle, and max separately for both SQLite and
  PostgreSQL serve setup.
- A read failure for each gauge source emits no metric datapoint and reports the
  fixed diagnostic context for that source.
- `docs/observability.md` and `docs/ARCHITECTURE.md` describe the delivered
  semantics without contradicting ADR-0011 or ADR-0058.

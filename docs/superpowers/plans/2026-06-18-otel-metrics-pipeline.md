# OTel Metrics Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emit a curated set of OpenTelemetry event counters/histograms for operational visibility, exported over the existing OTLP pipeline.

**Architecture:** A cardinality-safe emit facade in `common::metrics` (feature-gated, enum-typed helpers reading `global::meter`), plus an OTLP `MeterProvider` set up in `server::observability` alongside the existing tracer. Instruments are no-ops when no provider is installed, exactly like traces today.

**Tech Stack:** opentelemetry 0.30, opentelemetry_sdk 0.30 (`rt-tokio`, `metrics`, `testing`), opentelemetry-otlp 0.30 (`grpc-tonic`, `metrics`).

## Global Constraints

- opentelemetry crates are pinned at **0.30** via the workspace; add features, never bump versions.
- **Every metric attribute is a bounded enum** — never a username, id, URL, or free string. Helpers take Rust enums, not `&str`.
- Instrument names are dotted `jaunder.<area>.<thing>`; histograms set a unit (`By` for bytes, `ms` for durations).
- `common` must keep **zero `target_arch` cfgs** (jaunder-kq8w.10). The metrics code is **feature**-gated (`#[cfg(feature = "metrics")]`), never `target_arch`-gated; `opentelemetry` is an **optional** dep on `common`.
- The full catalog and emit-site mapping are in `docs/superpowers/specs/2026-06-18-otel-metrics-pipeline-design.md` — the authority for names/attributes.
- Commit after each task. Run `scripts/verify --fast` while iterating; the final task runs the full gate + re-baseline.

---

### Task 1: OTLP meter pipeline in `server::observability`

**Files:**
- Modify: `Cargo.toml` (workspace `opentelemetry_sdk`/`opentelemetry-otlp` features)
- Modify: `server/src/observability.rs` (add `build_otel_meter`, wire into `init_tracing_impl`)
- Test: `server/src/observability.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `fn build_otel_meter(endpoint: &str) -> Result<(), String>` (private); `init_tracing` now also installs the global `MeterProvider` when an OTLP endpoint is configured.

- [ ] **Step 1: Add the metrics features to the workspace deps**

In `Cargo.toml`:
```toml
opentelemetry_sdk = { version = "0.30.0", features = ["rt-tokio", "metrics", "testing"] }
opentelemetry-otlp = { version = "0.30.0", features = ["grpc-tonic", "metrics"] }
```

- [ ] **Step 2: Write the failing test** in `server/src/observability.rs` tests

```rust
#[tokio::test]
async fn build_otel_meter_accepts_valid_endpoint() {
    assert!(build_otel_meter("http://127.0.0.1:4317").is_ok());
}
```

- [ ] **Step 3: Run it, expect failure**

Run: `cargo test -p jaunder --lib observability::tests::build_otel_meter_accepts_valid_endpoint`
Expected: FAIL — `build_otel_meter` not found.

- [ ] **Step 4: Implement `build_otel_meter`** in `server/src/observability.rs` (next to `build_otel_tracer`)

```rust
fn build_otel_meter(endpoint: &str) -> Result<(), String> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| format!("failed to build OTLP metric exporter: {error}"))?;
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .build();
    opentelemetry::global::set_meter_provider(provider);
    Ok(())
}
```

- [ ] **Step 5: Wire it into `init_tracing_impl`** — after the `otel_layer` block, before `try_init`, install the meter on the same endpoint:

```rust
    // Metrics share the OTLP endpoint with traces; setup failure is non-fatal.
    if let Some(endpoint) = otel_exporter_otlp_endpoint() {
        if let Err(error) = build_otel_meter(&endpoint) {
            eprintln!("OTel metrics disabled because exporter setup failed (endpoint {endpoint}): {error}");
        }
    }
```

- [ ] **Step 6: Run the new test + the existing init tests**

Run: `cargo test -p jaunder --lib observability::`
Expected: PASS (including `build_otel_meter_accepts_valid_endpoint` and the existing `init_tracing_impl_*` tests, which now also exercise the meter branch).

- [ ] **Step 7: Add a branch test** mirroring the tracer's invalid-endpoint coverage:

```rust
#[tokio::test]
async fn build_otel_meter_with_endpoint_is_wired_by_init() {
    let _guard = lock_env();
    std::env::set_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4317");
    init_tracing_impl(false);
    std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
}
```

- [ ] **Step 8: `scripts/verify --fast`, then commit**

```bash
git add Cargo.toml Cargo.lock server/src/observability.rs
git commit -m "feat(server): install an OTLP MeterProvider alongside the tracer (kq8w.21)" -m "Refs: jaunder-kq8w.21" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: The `common::metrics` facade

**Files:**
- Modify: `common/Cargo.toml` (optional `opentelemetry` dep + `metrics` feature + dev-dep for the in-memory reader)
- Create: `common/src/metrics.rs`
- Modify: `common/src/lib.rs` (`#[cfg(feature = "metrics")] pub mod metrics;`)
- Test: `common/src/metrics.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Produces (all `#[cfg(feature = "metrics")]`): the emit helpers and their outcome enums used by every later task. Exact signatures defined below; later tasks consume them verbatim.

- [ ] **Step 1: Cargo wiring** in `common/Cargo.toml`

```toml
[dependencies]
opentelemetry = { workspace = true, optional = true }

[features]
test-utils = []
metrics = ["dep:opentelemetry"]

[dev-dependencies]
opentelemetry = { workspace = true }
opentelemetry_sdk = { workspace = true, features = ["metrics", "testing"] }
```
(Keep existing `[dependencies]`/`[dev-dependencies]` entries; add these.)

- [ ] **Step 2: Declare the module** in `common/src/lib.rs`

```rust
#[cfg(feature = "metrics")]
pub mod metrics;
```

- [ ] **Step 3: Write the facade** `common/src/metrics.rs` (instruments + enums + helpers). Full content:

```rust
//! Cardinality-safe OpenTelemetry metric emitters, shared by `web`, `server`,
//! and the CLI. Instruments are built once from the global meter; when no
//! `MeterProvider` is installed (no OTLP endpoint, or any non-server process)
//! they are no-ops. Helper arguments are bounded enums, so a call site can
//! never emit an unbounded attribute. See the design spec / ADR-0011.

use std::sync::LazyLock;

use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, KeyValue};

macro_rules! enum_attr {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug)]
        pub enum $name { $($variant),+ }
        impl $name {
            fn as_str(self) -> &'static str { match self { $(Self::$variant => $s),+ } }
        }
    };
}

enum_attr!(LoginOutcome { Success => "success", InvalidCredentials => "invalid_credentials", InternalError => "internal_error" });
enum_attr!(SessionOutcome { Ok => "ok", InvalidToken => "invalid_token", SessionNotFound => "session_not_found", Internal => "internal" });
enum_attr!(RegistrationSource { Web => "web", Cli => "cli" });
enum_attr!(RegistrationPolicy { Open => "open", InviteOnly => "invite_only", CliBypass => "cli_bypass" });
enum_attr!(RegistrationResult { Ok => "ok", Rejected => "rejected" });
enum_attr!(InviteEvent { Created => "created", Redeemed => "redeemed" });
enum_attr!(PasswordResetEvent { Requested => "requested", Completed => "completed" });
enum_attr!(EmailKind { Verification => "verification", PasswordReset => "password_reset" });
enum_attr!(SendResult { Success => "success", Failure => "failure" });
enum_attr!(UploadOutcome { Stored => "stored", Deduplicated => "deduplicated", QuotaExceeded => "quota_exceeded", TooLarge => "too_large", Invalid => "invalid" });
enum_attr!(ServeResult { Ok => "ok", NotFound => "not_found", NotModified => "not_modified" });
enum_attr!(RegenResult { Ok => "ok", Error => "error" });
enum_attr!(PingOutcome { Success => "success", Failed => "failed", Exhausted => "exhausted", NoHub => "no_hub" });
enum_attr!(CacheResult { Hit => "hit", Miss => "miss" });
enum_attr!(BackupResult { Success => "success", Failure => "failure" });
enum_attr!(PostEvent { Created => "created", Updated => "updated", Published => "published", Deleted => "deleted" });
enum_attr!(AtompubResult { Ok => "ok", ClientError => "client_error", ServerError => "server_error" });

struct Instruments {
    logins: Counter<u64>,
    session_validations: Counter<u64>,
    registrations: Counter<u64>,
    invites: Counter<u64>,
    password_resets: Counter<u64>,
    errors: Counter<u64>,
    email_sent: Counter<u64>,
    email_send_duration: Histogram<u64>,
    media_uploads: Counter<u64>,
    media_upload_bytes: Histogram<u64>,
    media_served: Counter<u64>,
    feed_regenerations: Counter<u64>,
    feed_regen_duration: Histogram<u64>,
    websub_pings: Counter<u64>,
    feed_cache: Counter<u64>,
    backup_runs: Counter<u64>,
    backup_duration: Histogram<u64>,
    backup_bytes: Histogram<u64>,
    backup_pruned: Counter<u64>,
    posts: Counter<u64>,
    atompub_requests: Counter<u64>,
}

static M: LazyLock<Instruments> = LazyLock::new(|| {
    let m = global::meter("jaunder");
    Instruments {
        logins: m.u64_counter("jaunder.auth.logins").build(),
        session_validations: m.u64_counter("jaunder.auth.session_validations").build(),
        registrations: m.u64_counter("jaunder.auth.registrations").build(),
        invites: m.u64_counter("jaunder.auth.invites").build(),
        password_resets: m.u64_counter("jaunder.auth.password_resets").build(),
        errors: m.u64_counter("jaunder.errors").build(),
        email_sent: m.u64_counter("jaunder.email.sent").build(),
        email_send_duration: m.u64_histogram("jaunder.email.send_duration").with_unit("ms").build(),
        media_uploads: m.u64_counter("jaunder.media.uploads").build(),
        media_upload_bytes: m.u64_histogram("jaunder.media.upload_bytes").with_unit("By").build(),
        media_served: m.u64_counter("jaunder.media.served").build(),
        feed_regenerations: m.u64_counter("jaunder.feed.regenerations").build(),
        feed_regen_duration: m.u64_histogram("jaunder.feed.regeneration_duration").with_unit("ms").build(),
        websub_pings: m.u64_counter("jaunder.feed.websub_pings").build(),
        feed_cache: m.u64_counter("jaunder.feed.cache").build(),
        backup_runs: m.u64_counter("jaunder.backup.runs").build(),
        backup_duration: m.u64_histogram("jaunder.backup.duration").with_unit("ms").build(),
        backup_bytes: m.u64_histogram("jaunder.backup.bytes").with_unit("By").build(),
        backup_pruned: m.u64_counter("jaunder.backup.pruned").build(),
        posts: m.u64_counter("jaunder.posts").build(),
        atompub_requests: m.u64_counter("jaunder.atompub.requests").build(),
    }
});

#[inline]
fn kv(key: &'static str, value: &'static str) -> [KeyValue; 1] {
    [KeyValue::new(key, value)]
}

pub fn login(outcome: LoginOutcome) { M.logins.add(1, &kv("outcome", outcome.as_str())); }
pub fn session_validation(outcome: SessionOutcome) { M.session_validations.add(1, &kv("outcome", outcome.as_str())); }
pub fn registration(source: RegistrationSource, policy: RegistrationPolicy, result: RegistrationResult) {
    M.registrations.add(1, &[
        KeyValue::new("source", source.as_str()),
        KeyValue::new("policy", policy.as_str()),
        KeyValue::new("result", result.as_str()),
    ]);
}
pub fn invite(event: InviteEvent) { M.invites.add(1, &kv("event", event.as_str())); }
pub fn password_reset(event: PasswordResetEvent) { M.password_resets.add(1, &kv("event", event.as_str())); }
pub fn error(kind: &'static str, class: &'static str) {
    M.errors.add(1, &[KeyValue::new("error.kind", kind), KeyValue::new("error.class", class)]);
}
pub fn email_sent(kind: EmailKind, result: SendResult) {
    M.email_sent.add(1, &[KeyValue::new("kind", kind.as_str()), KeyValue::new("result", result.as_str())]);
}
pub fn email_send_duration_ms(ms: u64) { M.email_send_duration.record(ms, &[]); }
pub fn media_upload(outcome: UploadOutcome) { M.media_uploads.add(1, &kv("outcome", outcome.as_str())); }
pub fn media_upload_bytes(bytes: u64) { M.media_upload_bytes.record(bytes, &[]); }
pub fn media_served(result: ServeResult) { M.media_served.add(1, &kv("result", result.as_str())); }
pub fn feed_regeneration(result: RegenResult) { M.feed_regenerations.add(1, &kv("result", result.as_str())); }
pub fn feed_regen_duration_ms(ms: u64) { M.feed_regen_duration.record(ms, &[]); }
pub fn websub_ping(outcome: PingOutcome) { M.websub_pings.add(1, &kv("outcome", outcome.as_str())); }
pub fn feed_cache(result: CacheResult) { M.feed_cache.add(1, &kv("result", result.as_str())); }
pub fn backup_run(result: BackupResult) { M.backup_runs.add(1, &kv("result", result.as_str())); }
pub fn backup_duration_ms(ms: u64) { M.backup_duration.record(ms, &[]); }
pub fn backup_bytes(bytes: u64) { M.backup_bytes.record(bytes, &[]); }
pub fn backup_pruned(count: u64) { M.backup_pruned.add(count, &[]); }
pub fn post(event: PostEvent) { M.posts.add(1, &kv("event", event.as_str())); }
pub fn atompub_request(op: &'static str, result: AtompubResult) {
    M.atompub_requests.add(1, &[KeyValue::new("op", op), KeyValue::new("result", result.as_str())]);
}
```

- [ ] **Step 4: Write the in-memory-reader test** (proves a helper records a value + attribute) in `metrics.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    #[tokio::test]
    async fn login_records_outcome_attribute() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        global::set_meter_provider(provider.clone());

        login(LoginOutcome::InvalidCredentials);
        provider.force_flush().expect("flush");

        let metrics = exporter.get_finished_metrics().expect("metrics");
        let found = metrics.iter().flat_map(|rm| rm.scope_metrics()).flat_map(|sm| sm.metrics())
            .any(|m| m.name() == "jaunder.auth.logins");
        assert!(found, "jaunder.auth.logins not exported");
    }
}
```
(If 0.30's `InMemoryMetricExporter` accessor names differ, adjust to the installed API — the assertion target is: the named instrument is exported after a helper call + flush.)

- [ ] **Step 5: Run the facade test**

Run: `cargo test -p common --features metrics metrics::tests`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add common/Cargo.toml Cargo.lock common/src/lib.rs common/src/metrics.rs
git commit -m "feat(common): add the cardinality-safe metrics emit facade (kq8w.21)" -m "Refs: jaunder-kq8w.21" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Enable the feature downstream + the error-boundary emit

**Files:**
- Modify: `web/Cargo.toml` (enable `common/metrics` under `ssr`)
- Modify: `server/Cargo.toml` (enable `common/metrics`)
- Modify: `web/src/error.rs` (emit `jaunder.errors` at the boundary)

**Interfaces:**
- Consumes: `common::metrics::error(kind, class)` from Task 2.

- [ ] **Step 1: Enable the feature.** In `web/Cargo.toml` add `common/metrics` to the `ssr` feature list; in `server/Cargo.toml` set `common = { workspace = true, features = ["metrics"] }`. (Confirm `web`'s `common` dep gains `metrics` only via `ssr`, never the wasm/hydrate default.)

- [ ] **Step 2: Find the single boundary** where `InternalError` is rendered to a response / logged (the `boundary!` macro or `into_public`/operator-message emit point in `web/src/error.rs`). Add, where `kind`/`class` are already in hand:

```rust
#[cfg(feature = "ssr")]
common::metrics::error(self.kind.as_metric_str(), self.class.as_metric_str());
```
(Add small `as_metric_str(&self) -> &'static str` mappers on `ErrorKind`/`ErrorClass` returning their bounded names — these are the same stable strings already emitted as span fields in §3.1bc.)

- [ ] **Step 3: Verify it fires** — extend an existing `error.rs` test (one that builds a `Server`/`Storage` `InternalError`) using the in-memory reader pattern from Task 2 to assert `jaunder.errors` is exported with the expected `error.kind`/`error.class`.

- [ ] **Step 4: `scripts/verify --fast`, then commit**

```bash
git add web/Cargo.toml server/Cargo.toml Cargo.lock web/src/error.rs
git commit -m "feat(web): emit jaunder.errors at the InternalError boundary (kq8w.21)" -m "Refs: jaunder-kq8w.21" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Auth & registration/invite/reset emit sites

**Files (modify):** `web/src/auth/server.rs`, the registration server fn (`web/src/auth/server.rs` `register`/`Register`), invite + password-reset server fns (`web/src/invites/`, `web/src/password_reset/`), `server/src/commands.rs` (CLI).

**Interfaces:** Consumes `common::metrics::{login, session_validation, registration, invite, password_reset, LoginOutcome, SessionOutcome, RegistrationSource, RegistrationPolicy, RegistrationResult, InviteEvent, PasswordResetEvent}`.

- [ ] **Step 1:** In `login_error`/the login path, emit `metrics::login(...)`: `Success` on a successful auth, `InvalidCredentials` for `UserAuthError::InvalidCredentials`, `InternalError` for `UserAuthError::Internal`. In `AuthUser::from_request_parts`, emit `metrics::session_validation(...)` mapping `Ok`/`SessionAuthError::{InvalidToken,SessionNotFound,Internal}`.
- [ ] **Step 2:** Registration server fn → `metrics::registration(RegistrationSource::Web, policy, result)` where `policy` is `Open`/`InviteOnly` from the active signup policy and `result` is `Ok` on success / `Rejected` on a policy or validation rejection; `cmd_user_create` → `metrics::registration(RegistrationSource::Cli, RegistrationPolicy::CliBypass, result)`. Invite create (web + `cmd_user_invite`) → `metrics::invite(InviteEvent::Created)`; invite redemption (in the register-with-invite path) → `InviteEvent::Redeemed`. Password-reset request/confirm → `metrics::password_reset(PasswordResetEvent::{Requested,Completed})`.
- [ ] **Step 3:** `scripts/verify --fast`.
- [ ] **Step 4: Commit** (`feat: emit auth/registration/invite/reset metrics (kq8w.21)`).

---

### Task 5: Email emit sites

**Files (modify):** `web/src/email/` (verification send) and `web/src/password_reset/` (reset send) server fns.

**Interfaces:** Consumes `common::metrics::{email_sent, email_send_duration_ms, EmailKind, SendResult}`.

- [ ] **Step 1:** Around each `MailSender::send_*` call, time it (`std::time::Instant`), then emit `metrics::email_sent(kind, result)` and `metrics::email_send_duration_ms(elapsed)` — `kind` = `Verification` or `PasswordReset` (known at the call site), `result` from the send `Result`.
- [ ] **Step 2:** `scripts/verify --fast`. **Commit** (`feat: emit email send metrics (kq8w.21)`).

---

### Task 6: Media emit sites

**Files (modify):** `server/src/media_manager.rs`, `server/src/media.rs`, `server/src/atompub/media.rs`.

**Interfaces:** Consumes `common::metrics::{media_upload, media_upload_bytes, media_served, UploadOutcome, ServeResult}`.

- [ ] **Step 1:** In `MediaManager::finalize_upload`/`upload_bytes`, emit `media_upload(Stored|Deduplicated)` on success (distinguish via `handle_deduplication`) and `media_upload_bytes(size)`; on the error paths emit `QuotaExceeded`/`TooLarge`/`Invalid` mapping `MediaError`. In `media.rs` `serve_handler`, emit `media_served(Ok|NotFound|NotModified)`.
- [ ] **Step 2:** `scripts/verify --fast`. **Commit** (`feat: emit media upload/serve metrics (kq8w.21)`).

---

### Task 7: Feed worker & cache emit sites

**Files (modify):** `server/src/feed/worker.rs`, `server/src/feed/handlers.rs`.

**Interfaces:** Consumes `common::metrics::{feed_regeneration, feed_regen_duration_ms, websub_ping, feed_cache, RegenResult, PingOutcome, CacheResult}`.

- [ ] **Step 1:** In `FeedWorker::tick`, on each regenerate: `feed_regeneration(Ok|Error)` + `feed_regen_duration_ms(started.elapsed())` (the timer already exists). For pings: `websub_ping(Success|Failed|Exhausted|NoHub)` mapping the existing ping outcome arms. In `feed/handlers.rs::serve`, emit `feed_cache(Hit)` on `Ok(Some(..))` and `Miss` on `Ok(None)`.
- [ ] **Step 2:** `scripts/verify --fast`. **Commit** (`feat: emit feed regen/websub/cache metrics (kq8w.21)`).

---

### Task 8: Backup emit sites

**Files (modify):** `server/src/backup.rs`.

**Interfaces:** Consumes `common::metrics::{backup_run, backup_duration_ms, backup_bytes, backup_pruned, BackupResult}`.

- [ ] **Step 1:** In `run_scheduled_backup_logged`, time the run, emit `backup_run(Success|Failure)` + `backup_duration_ms(elapsed)`; on success record `backup_bytes(written_size)` (sum the manifest/archive size) and `backup_pruned(n)` (return/forward the prune count from `prune_backups`). The same emits cover both the scheduled job and `cmd_backup` if both route through the helper.
- [ ] **Step 2:** `scripts/verify --fast`. **Commit** (`feat: emit backup run metrics (kq8w.21)`).

---

### Task 9: Posts & AtomPub emit sites

**Files (modify):** `web/src/posts/server.rs`, `server/src/atompub/{posts,media,service,rsd}.rs` (via the `HandlerError` boundary).

**Interfaces:** Consumes `common::metrics::{post, atompub_request, PostEvent, AtompubResult}`.

- [ ] **Step 1:** In the post create/update/publish/delete server fns, emit `post(PostEvent::*)` on success. For AtomPub, emit `atompub_request(op, result)` once per handler — cleanest at the handlers' single boundary: in `HandlerError::into_response` you only know the status, so instead emit at the end of each handler with the handler's static `op` name and `AtompubResult` derived from the outcome (Ok vs the `HandlerError` 4xx/5xx class). Use the existing `#[tracing::instrument(name = "...")]` names as the `op` values.
- [ ] **Step 2:** `scripts/verify --fast`. **Commit** (`feat: emit posts/atompub metrics (kq8w.21)`).

---

### Task 10: Docs + coverage re-baseline + full gate

**Files:** `docs/decisions/0011-unified-observability.md` (addendum), `.coverage-manifest.json`, `.crap-manifest.json`.

- [ ] **Step 1:** Add an ADR-0011 addendum: the metric catalog, naming/bounded-attribute conventions, the `common/metrics` feature + env-driven enablement (shares the OTLP endpoint with traces), and the in-memory-reader test pattern. Note the deferred slices (gauges → kq8w.24, CLI export → kq8w.25).
- [ ] **Step 2:** Regenerate the baseline: `scripts/update-coverage-baseline`, then review the diff (expected: new covered lines in `observability.rs`, `common/src/metrics.rs`, and the emit-site files; no unexplained drops).
- [ ] **Step 3:** Run the full gate via context-mode: `ctx_execute(language: "shell", code: "scripts/verify")`. Expected: `commit gate passed`.
- [ ] **Step 4: Commit** the docs + manifests together:

```bash
git add docs/decisions/0011-unified-observability.md .coverage-manifest.json .crap-manifest.json
git commit -m "docs: document the metrics pipeline + re-baseline coverage (kq8w.21)" -m "Refs: jaunder-kq8w.21" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 5:** `bd close jaunder-kq8w.21`.

---

## Notes for the implementer

- **OTLP 0.30 API drift:** Task 1/2 pin the exact builder/accessor names; if a method (`with_periodic_exporter`, `InMemoryMetricExporter` accessors) differs in the installed 0.30 point release, adjust to compile — the *shape* (exporter → reader/provider → `set_meter_provider`; flush → read finished metrics) is stable.
- **Feature unification:** `common::metrics` (and its tests) compile with the `metrics` feature ON during a `--workspace`/nextest build because `server` activates `common/metrics` and cargo unifies workspace features. Confirm in Task 2 Step 5 by running the facade test as part of `cargo test -p common --features metrics` and again via the workspace build in Task 10's gate.
- **No double-count:** emit each event at exactly one layer (e.g. email at the web call site, never also in the mailer).
- **Coverage discipline:** keep every emit line on an already-tested path so it stays covered; the re-baseline should show additions/improvements, not drops.
```

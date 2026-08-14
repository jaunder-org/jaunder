# Issue #58 Error-Swallowing Audit Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> `jaunder-iterate`; delegate an individual task via `jaunder-dispatch`. Steps
> use checkbox (`- [ ]`) syntax for tracking.

## Review

**Goal:** Preserve every unexpected production failure or report it before an
intentional continuation, including bounded native/client diagnostics and the
explicit UI failures absorbed from #898 and #899.

**Scope:**

- In: every shipping Rust target in the root workspace, `xtask/`, and `tools/`;
  storage claim classification; native and browser swallowed-error reporting;
  authenticated/rate-limited client intake; startup, media, projector, runtime,
  and developer-tool corrections; #898/#899 UI states; checked-in audit
  evidence.
- Out: test-only/fixture cleanup, a syntax allowlist gate, browser
  OpenTelemetry, arbitrary log ingestion, durable client queues, direct OTLP,
  and renaming `jaunder.errors`.

**Task map:**

1. Materialize the complete classified source inventory and accepted policy
   docs.
2. Repair typed invite/email/password claims and all seven password/account
   source-erasing mappings.
3. Add the atomic native swallowed reporter and independent proofs for all
   fourteen observability/storage-threshold amendments plus panic fallback.
4. Make SMTP/database startup and credential resolution fail closed while
   preserving all thirteen sources.
5. Propagate AtomPub/feed/media failures and report only projector
   continuations, covering all eleven amendments plus PageSeed proof.
6. Preserve two WebSub sources and report all 25 true native continuation
   amendments once at their useful aggregation boundary.
7. Add the bounded client event contract and host-testable one-flight reporter.
8. Add the cookie-only intake, bounded limiter, and backend-parametric HTTP
   tests.
9. Wire audited browser failures to the reporter and prove real end-to-end
   delivery.
10. Replace fabricated profile-format and audience-picker state with explicit
    failure.
11. Make server-build/`xtask`/`devtool` correctness paths fail closed and
    ancillary failures warn, with exact proofs for all sixteen tool amendments.
12. Mechanically reconcile all 88 remediation keys, evidence, and the full
    repository gate.

**Key risks/decisions:**

- The audit manifest, not syntax totals, is the completion proof. Production
  edits do not begin until its initial population is committed.
- `host::error::report_swallowed` is the only native warning+metric seam. The
  metric labels stay closed; per-site context remains a tracing field.
- Client telemetry is untrusted diagnostics. Its wire carries only closed enums,
  authenticates only the `session=` cookie, and cannot recursively report
  failure.
- The limiter is process-local and intentionally resets on restart. Cleanup is
  bounded to 64 ring entries per intake attempt.
- UI fetch failures are not swallowed; they become visible state and therefore
  do not also emit client-swallow telemetry.
- `xtask`/`devtool` do not gain an OTel runtime. Correctness fails; legitimate
  cleanup/diagnostic loss warns on stderr without replacing the primary result.
- The final inventory gate validates the checked-in manifest and 88-key
  delivered-remediation ledger only. It is not a source-syntax allowlist.

**Architecture:** Put the cross-target, closed-enum request in `common`; keep
the host-testable reporting state machine and wasm `fetch` adapter in `client`;
convert accepted reports to the existing tracing/OTel pipeline in a focused raw
Axum module. Native callers use one `host::error` reporter, while developer
tools use `Result` or fixed stderr warnings according to whether continuation is
valid.

**Tech stack:** Rust 2024, sqlx SQLite/PostgreSQL, Axum 0.8, Leptos 0.8,
`web-sys`, tracing/OpenTelemetry 0.30, `rstest_reuse`, cargo-nextest,
Playwright, and the repository `cargo xtask` gate.

## Global Constraints

- Apply the semantic rule from the approved spec: expected validation/domain
  rejection may become control flow; unexpected infrastructure, I/O, browser,
  subprocess, invariant, or decode failure must propagate or be reported before
  intentional continuation.
- Audit every non-test Rust target in the root workspace, `xtask/`, and
  `tools/`; exclude `#[cfg(test)]` modules, test targets, fixtures, benches, and
  rustdoc.
- Do not add a broad scanner/allowlist gate for `.ok()`, `unwrap_or`, `let _`,
  `Err(_)`, or `map_err`.
- Native reporting emits one fixed WARN and one `jaunder.errors` measurement
  atomically; diagnostic self-failure uses only its fallback sink.
- `jaunder.errors` labels are bounded:
  `error.disposition = boundary | swallowed` and
  `telemetry.origin = server | client`; static context is never a metric label.
- Client wire types contain no free-form source, URL, route, username,
  identifier, form value, or request body.
- Client intake accepts one JSON event of at most 1,024 encoded bytes, only from
  a valid `session=` cookie. Bearer and Basic/app-password authentication do not
  authorize it.
- Client rate limiting is per user: burst 5, refill 1/minute, full-bucket stale
  timeout 15 minutes, at most 64 ring entries visited per request, exactly one
  ring entry per bucket.
- Client delivery logs locally first, permits one request in flight, uses
  credentialed `fetch` with `keepalive: true`, and has no queue, retry,
  persistence, browser OTel SDK, or direct OTLP.
- All storage behavior tests that apply to both backends use the repository
  `#[apply(backends)]` template and `Backend::setup()` fixture.
- Do not add lint suppressions. Do not add a `Co-Authored-By` trailer.
- Before every task commit, run the full `devtool run -- cargo xtask check`
  after all focused tests for that task pass.
- Run executable/gate commands through `devtool run --` with pinned binaries;
  source-inventory searches use the pinned bare `rg` per repository routing.

---

### Task 1: Classified Audit Manifest and Policy Baseline

**Files:**

- Create:
  `docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-inventory.md`
- Modify: `docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-audit.md`
- Modify: `docs/adr/0011-unified-observability.md`
- Modify: `docs/adr/0017-error-handling-and-the-public-boundary.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `CONTRIBUTING.md`
- Track: `docs/superpowers/plans/2026-08-13-issue-58-error-swallowing-audit.md`

**Interfaces:**

- Consumes: approved decisions D1-D11 and AC1/AC21.
- Produces: one authoritative inventory row per non-test hit, keyed by
  `(path, containing symbol, expression)`, with disposition `expected`,
  `propagated`, or `continued`. Every `continued` row also names static context,
  reporting site, reason, and behavioral proof.

- [x] **Step 1: Record reproducible population recipes**

  Run these exact PCRE2 searches over the same roots for each family (source
  search uses the pinned bare `rg`, not `devtool run`):

  ```bash
  rg -n --pcre2 --type rust --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/fixtures/**' --glob '!**/examples/**' '(?:\.ok\(\)|Result::ok)' common/src client/src csr/src host/src macros/src server/src storage/src web/src xtask/src tools/devtool/src server/build.rs tools/doctests/src
  rg -n --pcre2 --type rust --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/fixtures/**' --glob '!**/examples/**' '(?:unwrap_or(?:_default|_else)?|map_or(?:_else)?)\(' common/src client/src csr/src host/src macros/src server/src storage/src web/src xtask/src tools/devtool/src server/build.rs tools/doctests/src
  rg -n --pcre2 --type rust --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/fixtures/**' --glob '!**/examples/**' '(?:let\s+_\s*=|drop\()' common/src client/src csr/src host/src macros/src server/src storage/src web/src xtask/src tools/devtool/src server/build.rs tools/doctests/src
  rg -n --pcre2 --type rust --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/fixtures/**' --glob '!**/examples/**' '(?:Err\(\s*_\s*\)|Err\(\s*\.\.\s*\)|_\s*=>)' common/src client/src csr/src host/src macros/src server/src storage/src web/src xtask/src tools/devtool/src server/build.rs tools/doctests/src
  rg -n -U --pcre2 --type rust --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/fixtures/**' --glob '!**/examples/**' 'map_err\(\s*\|[^|]*\|' common/src client/src csr/src host/src macros/src server/src storage/src web/src xtask/src tools/devtool/src server/build.rs tools/doctests/src
  rg -n -U --pcre2 --type rust --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/fixtures/**' --glob '!**/examples/**' 'Err\([^)]*\)\s*=>' common/src client/src csr/src host/src macros/src server/src storage/src web/src xtask/src tools/devtool/src server/build.rs tools/doctests/src
  rg -n --pcre2 --type rust --glob '!**/tests/**' --glob '!**/benches/**' --glob '!**/fixtures/**' --glob '!**/examples/**' '(?:use_invite|use_email_verification|confirm_password_reset|build_mailer|serve_response|permalink_response|timeline_response|tag_response|mark_regenerated|mark_pinged|mark_exhausted|mark_failed|backup_size_bytes|remove_runtime_file|WorktreeGuard)' common/src client/src csr/src host/src macros/src server/src storage/src web/src xtask/src tools/devtool/src server/build.rs tools/doctests/src
  ```

  The manifest records each command verbatim and its raw count. From each raw
  result, exclude a hit only when its containing syntax node is inside
  `#[cfg(test)]`, a test/bench/example target, a fixture subtree, or a rustdoc
  comment/code fence; record excluded counts by recipe and reason. Resolve the
  containing symbol, normalize only whitespace in the quoted expression, and
  deduplicate the union by `(path, containing symbol, normalized expression)`,
  retaining every recipe family on a multi-family row. Record both the
  per-family included totals and the deduplicated union total.

- [x] **Step 2: Classify every initial hit before production edits**

  Each row quotes the actual expression and states why the operation is
  expected, already propagated, or intentionally continued. Do not use aggregate
  counts as a substitute. Include every known high-risk site in storage claims,
  mailer startup, media open, development auto-init, projector responses,
  scheduled feed/backup work, runtime-file/capture cleanup, ADR/coverage
  populations, diagnostic artifact handling, ephemeral PostgreSQL teardown, and
  browser APIs.

- [x] **Step 3: Validate the policy projection**

  Confirm ADR-0017 defines swallow semantically and requires
  propagate-or-report; ADR-0011 defines disposition/origin and bounded client
  intake; `docs/ARCHITECTURE.md` projects both; `CONTRIBUTING.md` gives the
  actionable native/client/tool rule; `CONTEXT.md` remains unchanged.

- [x] **Step 4: Format and run the documentation/static gate**

  Run:

  ```bash
  devtool run -- prettier -w docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-audit.md docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-inventory.md docs/superpowers/plans/2026-08-13-issue-58-error-swallowing-audit.md docs/adr/0011-unified-observability.md docs/adr/0017-error-handling-and-the-public-boundary.md docs/ARCHITECTURE.md CONTRIBUTING.md
  devtool run -- cargo xtask check
  ```

  Expected: PASS; the manifest population is complete and all ADR/view parity
  checks remain green.

- [x] **Step 5: Commit the approved baseline**

  ```bash
  git add docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-audit.md docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-inventory.md docs/superpowers/plans/2026-08-13-issue-58-error-swallowing-audit.md docs/adr/0011-unified-observability.md docs/adr/0017-error-handling-and-the-public-boundary.md docs/ARCHITECTURE.md CONTRIBUTING.md
  git commit -m "docs: define error swallowing audit contract"
  ```

### Task 2: Typed Storage, Password, and Account-Command Failures

**Files:**

- Modify: `common/src/password.rs`
- Modify: `storage/src/invites.rs`
- Modify: `storage/src/email.rs`
- Modify: `storage/src/atomic.rs`
- Modify: `storage/src/helpers.rs`
- Modify: `storage/src/postgres/atomic.rs`
- Modify: `storage/src/postgres/mod.rs`
- Modify: `storage/src/sqlite/atomic.rs`
- Modify: `storage/src/sqlite/mod.rs`
- Modify: `storage/src/users.rs`
- Modify: `server/src/commands.rs`
- Test: in-file unit tests in `common/src/password.rs` and
  `server/src/commands.rs`
- Test: `server/tests/misc/commands.rs`
- Modify: `server/tests/storage/mod.rs`
- Modify: `xtask/src/steps/sqlx_newtype_decode_check.rs`

**Interfaces:**

- Consumes:
  `EmailVerificationStorage::use_email_verification(&RawToken) -> Result<(UserId, Email), UseEmailVerificationError>`
  and
  `AtomicOps::confirm_password_reset(&RawToken, &Password) -> Result<(), ConfirmPasswordResetError>`.
- Produces: no `InviteStorage::use_invite` or `UseInviteError`; every failed
  sqlx operation in email verification becomes
  `UseEmailVerificationError::Internal`; password reset retains its
  implementation but gains a valid-token storage-failure proof.
- `PasswordError::HashingFailed` and `PasswordError::VerificationFailed` retain
  the typed `argon2::password_hash::Error` as their `#[source]`. Only
  `argon2::password_hash::Error::Password` is the expected `Ok(false)` mismatch;
  every other Argon2 error propagates.
- Account-command mappings use `anyhow::Context` and retain the original
  storage/crypto error. This task owns all seven remaining Task 2 inventory
  amendments: the four `common/src/password.rs` hash/verify mappings and
  `server/src/commands.rs` account mappings at lines 219, 247, and 255.

- [x] **Step 1: Write the backend-parametric claim regression tests**

  In `storage/src/email.rs`, add
  `use_email_verification_with_closed_pool_returns_internal`; seed a
  syntactically valid verification token, close `env.base`, call the method, and
  assert `matches!(result, Err(UseEmailVerificationError::Internal(_)))` for
  SQLite and PostgreSQL.

  In `storage/src/atomic.rs`, replace the malformed-token closed-pool setup with
  a freshly generated valid `RawToken`, close the pool, and assert
  `ConfirmPasswordResetError::Internal(_)`. Keep the existing NotFound, Expired,
  and AlreadyUsed cases unchanged.

- [x] **Step 2: Add typed-source red tests for all seven amendments**

  Add narrow private hash/verify operation parameters in `common::password` and
  the storage helper/atomic/auth call paths without changing any public
  signature. Production adapters call the current Argon2 methods directly.
  Inject a non-`Password` Argon2 failure through password reset and
  authentication separately; assert the same concrete source remains
  downcastable through `PasswordError`, `io::Error`, and the returned storage
  boundary. Assert `Error::Password` alone remains `Ok(false)`.

  Inject storage/crypto failures into `cmd_user_create` and both fallible
  operations in `app_password_create`; assert each returned `anyhow::Error`
  retains the concrete source while preserving the current human context. These
  tests pin the four password mappings plus command lines 219, 247, and 255.

- [x] **Step 3: Run the focused red tests**

  ```bash
  devtool run -- cargo nextest run -p common password
  devtool run -- cargo nextest run -p storage use_email_verification_with_closed_pool_returns_internal
  devtool run -- cargo nextest run -p storage confirm_password_reset_with_closed_pool_returns_error
  devtool run -- cargo nextest run -p storage password_source_chain
  devtool run -- cargo nextest run -p jaunder typed_account_command_source
  ```

  Expected before implementation: the source assertions fail because the
  password and command mappings stringify errors; email fails with `NotFound`;
  the password-reset test reaches storage instead of proving only malformed
  token rejection.

- [x] **Step 4: Remove the obsolete invite claim surface cleanly**

  Delete the trait method, error enum, generic implementation, dedicated
  `use_invite_*` tests, imports, and any explicit re-export. Use LSP references
  before deletion; the only registration path left is atomic
  `create_user_with_invite`.

- [x] **Step 5: Preserve every typed source**

  Keep token-hash validation as the expected `NotFound` path. Change both email
  query error mappings—the atomic claim and the disambiguation read—to
  `UseEmailVerificationError::Internal`; call `email_verification_claim_error`
  only for successful `Ok(None)` claim results. Replace password string payloads
  with source-carrying variants and replace the three account-command
  `anyhow!("{e}")` mappings with `anyhow::Context`; do not stringify at an
  intermediate layer.

- [x] **Step 6: Run the complete claim and password contract**

  ```bash
  devtool run -- cargo nextest run -p common password
  devtool run -- cargo nextest run -p storage email_verification
  devtool run -- cargo nextest run -p storage confirm_password_reset
  devtool run -- cargo nextest run -p storage create_user_with_invite
  devtool run -- cargo nextest run -p storage password_source_chain
  devtool run -- cargo nextest run -p jaunder typed_account_command_source
  devtool run -- cargo xtask check
  ```

  Expected: PASS on SQLite and PostgreSQL; token-domain variants remain
  unchanged; all seven concrete sources remain downcastable; no
  declaration/reference to `UseInviteError` or `use_invite` remains.

- [x] **Step 7: Commit**

  ```bash
  git add common/src/password.rs storage/src/invites.rs storage/src/email.rs storage/src/atomic.rs storage/src/helpers.rs storage/src/postgres/atomic.rs storage/src/postgres/mod.rs storage/src/sqlite/atomic.rs storage/src/sqlite/mod.rs storage/src/users.rs server/src/commands.rs server/tests/misc/commands.rs server/tests/storage/mod.rs xtask/src/steps/sqlx_newtype_decode_check.rs
  git commit -m "fix(storage): preserve typed claim and password failures"
  ```

### Task 3: Native Error Event Reporting

**Files:**

- Modify: `host/src/metrics.rs`
- Modify: `host/src/error.rs`
- Modify: `web/src/error/server.rs`
- Modify: `web/src/error/mod.rs`
- Modify: `server/src/observability.rs`
- Modify: `storage/src/db.rs`

**Interfaces:**

- Produces:

  ```rust
  enum ErrorDisposition {
      Boundary,
      Swallowed,
  }
  enum TelemetryOrigin {
      Server,
      Client,
  }
  pub enum SwallowedSource<'a> {
      Error(&'a (dyn std::error::Error + 'static)),
      Redacted,
  }
  pub fn report_swallowed(
      kind: ErrorKind,
      class: ErrorClass,
      context: &'static str,
      source: SwallowedSource<'_>,
  );
  fn record_error(
      kind: ErrorKind,
      class: ErrorClass,
      disposition: ErrorDisposition,
      origin: TelemetryOrigin,
  );
  ```

  `record_error` is private inside `host::error`; the existing free
  `host::metrics::error` entry point is removed. Only
  `InternalError::emit_boundary_failure()` and the atomic reporting functions
  can increment `jaunder.errors`. The boundary method calls
  `record_error(..., Boundary, Server)`; `report_swallowed` emits the fixed WARN
  fields and calls `record_error(..., Swallowed, Server)` exactly once.

- Observability setup/teardown self-failures never use `report_swallowed`.
  Diagnostic-file open, tracer exporter construction, meter exporter
  construction, meter shutdown, tracer shutdown, subscriber installation, and
  panic diagnostic fallback each have an independent fixed-stderr proof with
  zero recursive `jaunder.errors`.
- Tracer/meter exporter builders preserve their typed OTLP errors with context.
  Optional observability environment variables distinguish `NotPresent` from
  invalid Unicode or invalid configured values: only absence uses a silent
  default; configured-invalid input uses a fixed, value-redacted stderr warning
  and the documented fallback without recursive metrics.
- This task owns all fourteen Task 3 amendments: the two typed exporter-builder
  mappings and twelve continuations covering diagnostic open, endpoint/filter/
  format/threshold configuration, SQL threshold configuration, both shutdowns,
  and subscriber installation. The panic fallback remains an additional
  non-recursion proof.

- [x] **Step 1: Add metric-label tests**

  Extend the existing in-memory OTel tests to assert a boundary failure produces
  one `jaunder.errors` data point with `error.kind`, `error.class`,
  `error.disposition=boundary`, and `telemetry.origin=server`. Add exhaustive
  enum mapping assertions for both new labels.

- [x] **Step 2: Add swallowed reporter event+metric tests**

  Use a PII-safe test error and captured tracing subscriber. Call
  `report_swallowed(ErrorKind::Storage, ErrorClass::Transient, "server.test.cleanup", SwallowedSource::Error(&source))`;
  assert exactly one WARN with `error.kind`, `error.class`,
  `error.disposition="swallowed"`, `telemetry.origin="server"`, static
  `error.context`, and source. Assert exactly one metric with
  `swallowed/server`. Add a bounded-source-kind case proving no arbitrary text
  field is required.

- [x] **Step 3: Add fifteen independent observability source/non-recursion
      tests**

  Introduce narrow private operation adapters for diagnostic open, tracer/meter
  exporter construction, tracer/meter shutdown, and environment reads;
  production adapters call the current APIs directly. Do not add a generic
  reporting abstraction. Exercise each branch separately:
  1. diagnostic-file open failure: startup continues, one
     `server.observability.diag_log_open` stderr fallback, zero error metrics;
  2. tracer exporter construction failure: the concrete OTLP builder source
     remains in the error chain reaching one
     `server.observability.tracer_exporter_setup` fallback, startup continues,
     and zero error metrics are recorded;
  3. meter exporter construction failure: the equivalent typed-source and
     fallback assertions for `server.observability.meter_exporter_setup`;
  4. meter shutdown failure: dropping the guard preserves the process result,
     writes one `server.observability.meter_shutdown` line, and records zero
     error metrics;
  5. tracer shutdown failure: the same assertions for
     `server.observability.tracer_shutdown`;
  6. preinstalled subscriber: run an isolated subprocess because the global
     subscriber is process-wide; assert initialization returns, writes one
     `server.observability.subscriber_install` line, and records zero metrics;
  7. panic diagnostic-writer failure: assert the original chained panic hook
     runs once, only the fixed panic fallback is written, and zero metrics;
  8. invalid-Unicode primary OTLP endpoint: startup continues, the secondary
     endpoint is not silently selected, one redacted
     `server.observability.otlp_endpoint` line, and zero metrics;
  9. invalid-Unicode secondary OTLP endpoint: startup continues with export
     disabled, one redacted endpoint line, and zero metrics;
  10. invalid directive and invalid-Unicode `JAUNDER_LOG_FILTER`/`RUST_LOG`
      subcases: default filter/startup remain, one
      `server.observability.log_filter` warning per attempt, zero metrics;
  11. nonnumeric `JAUNDER_SLOW_OP_MS`: five-second fallback, one
      `server.observability.slow_threshold` warning, zero metrics;
  12. invalid-Unicode `JAUNDER_SLOW_OP_MS`: the same fallback and one redacted
      slow-threshold warning;
  13. invalid-Unicode `JAUNDER_LOG_FORMAT`: pretty output/startup remain, one
      redacted `server.observability.log_format` warning, zero metrics;
  14. nonnumeric `JAUNDER_SQL_SLOW_MS`: five-second fallback, one
      `storage.observability.sql_slow_threshold` warning, zero metrics; and
  15. invalid-Unicode `JAUNDER_SQL_SLOW_MS`: the same fallback and one redacted
      SQL-threshold warning.

  Capture stderr independently so one branch cannot stand in for another. Cases
  1-6 and 8-15 close all fourteen amendments; case 7 retains the separate panic
  fallback contract.

- [x] **Step 4: Record the red-phase disposition**

  ```bash
  devtool run -- cargo nextest run -p host error
  devtool run -- cargo nextest run -p web --features server boundary_failure
  devtool run -- cargo nextest run -p jaunder observability
  devtool run -- cargo nextest run -p storage sql_slow_query_threshold
  ```

  Expected before implementation: disposition/origin and typed exporter errors
  do not exist, and the fifteen independent assertions do not all pass.

  Resume note: implementation was already present when the controller resumed,
  so no truthful pre-implementation run remained. Reverting production code
  solely to manufacture a red result would not recover that sequencing evidence;
  Step 6 supplies the delivery evidence.

- [x] **Step 5: Implement the closed metric enums and atomic reporter**

  Keep kind/class conversion private to `host::error`; do not accept
  caller-provided metric strings. Render a reviewed typed source once. Keep
  per-site context on the tracing event only. Keep all diagnostic/configuration
  self-failures on fixed fallback sinks and out of this reporter.

- [x] **Step 6: Migrate the existing boundary emitter and verify**

  ```bash
  devtool run -- cargo nextest run -p host error
  devtool run -- cargo nextest run -p web --features server boundary_failure
  devtool run -- cargo nextest run -p jaunder observability
  devtool run -- cargo nextest run -p storage sql_slow_query_threshold
  devtool run -- cargo xtask check
  ```

  Expected: PASS; boundary and swallowed events each increment exactly once;
  both exporter sources remain typed; all twelve amendment continuations plus
  the panic fallback preserve their primary outcome and produce zero recursive
  error measurements.

- [x] **Step 7: Commit**

  ```bash
  git add host/src/error.rs host/src/metrics.rs web/src/error/mod.rs web/src/error/server.rs server/src/observability.rs storage/src/db.rs
  git commit -m "feat(host): report swallowed error events atomically"
  ```

### Task 4: Startup, SMTP, and CLI Source Preservation

**Files:**

- Modify: `server/src/mailer/factory.rs`
- Modify: `server/src/mailer/smtp.rs`
- Modify: `server/src/commands.rs`
- Modify: `storage/src/postgres/open.rs`
- Test: in-file unit tests in all four modified modules
- Test: `server/tests/misc/commands.rs`
- Test: `server/tests/misc/postgres/commands.rs`
- Test: `server/tests/storage/mod.rs`

**Interfaces:**

- `build_mailer(...)` returns `anyhow::Result<Arc<dyn MailSender>>`: absent SMTP
  configuration is `Ok(NoopMailSender)`; config read failure and
  present-but-invalid configuration are `Err` with their typed causes.
- `BuildMailerError::InvalidSender` retains `lettre::address::AddressError`;
  `BuildMailerError::Transport` retains the concrete lettre transport source.
  Command context is attached with `anyhow::Context`, never by interpolation.
- Development auto-init is a pure classification over backend, open error, and
  SQLite filename metadata. It returns true only for SQLite CANTOPEN code 14
  plus metadata `NotFound`; classification itself never mutates storage.
- PostgreSQL credential resolution distinguishes absent environment variables
  from invalid Unicode. `JAUNDER_DB_PASSWORD_FILE` and `JAUNDER_DB_PASSWORD`
  configured with invalid bytes fail closed with bounded, non-secret context;
  they never fall through to another credential source or render secret bytes.
- This task owns all thirteen Task 4 inventory amendments: command mappings at
  lines 200, 271, 289, 319, 323, 327, 345, 513, and 520; SMTP construction
  mappings at lines 65 and 79/83; and both PostgreSQL password environment
  paths.

- [x] **Step 1: Add mailer construction and source-chain tests**

  Pin absent config => Noop; injected config read error => `Err` with its
  source; syntactically present but invalid sender => `Err` retaining
  `AddressError`; injected StartTLS/TLS builder error => `Err` retaining the
  concrete transport source. Use narrow private construction adapters where
  lettre otherwise offers no deterministic failure input. Assert startup callers
  propagate rather than substitute Noop.

- [x] **Step 2: Add CLI source-chain tests for all nine command mappings**

  Inject database-open/configuration errors through `cmd_user_create`,
  `cmd_user_invite`, `cmd_app_password_create`, `cmd_smtp_test`, and both
  failure exits of `prepare_server`; assert each concrete error remains
  downcastable through its `anyhow` chain and the existing
  `run jaunder init first` context remains in `Display`.

  Inject invalid SMTP sender, transport-build failure, and send failure through
  `cmd_smtp_test` separately. Assert the address, transport, or send source
  remains downcastable. These tests name all nine command sites explicitly:
  lines 200, 271, 289, 319, 323, 327, 345, 513, and 520.

- [x] **Step 3: Add pure database auto-init classification tests**

  Construct cases for: SQLite CANTOPEN + missing filename => initialize; SQLite
  CANTOPEN + existing file => propagate; metadata permission/other error =>
  propagate; non-CANTOPEN SQLite error => propagate; malformed URL => propagate;
  migration error => propagate; PostgreSQL SQLSTATE 3D000 => propagate with
  `create-pg-db` guidance; representative PostgreSQL connection error =>
  propagate.

  Inject `VarError::NotUnicode` for `JAUNDER_DB_PASSWORD_FILE` and
  `JAUNDER_DB_PASSWORD` separately; assert option resolution fails with the
  typed environment source and bounded context, does not render bytes, and does
  not fall through. Assert `NotPresent` alone retains the documented fallback.

- [x] **Step 4: Run and observe current failures**

  ```bash
  devtool run -- cargo nextest run -p jaunder mailer_source_chain
  devtool run -- cargo nextest run -p jaunder command_source_chain
  devtool run -- cargo nextest run -p jaunder auto_init
  devtool run -- cargo nextest run -p storage postgres_password_from_env
  ```

  Expected before implementation: mailer/startup sources are stringified,
  PostgreSQL configured-invalid credentials fall through, and at least the
  malformed/open classification assertions fail.

  Red-phase disposition: the delegated implementation was already present before
  the controller could run these focused tests. The first controller runs
  exposed test compilation defects and an unbounded PostgreSQL connection case,
  but no truthful pre-implementation behavior run remained. Those defects were
  repaired; Step 6 supplies the delivery evidence.

- [x] **Step 5: Implement fail-closed startup and typed propagation**

  Replace all eleven string-erasing mappings with typed variants or
  `anyhow::Context`; never inspect formatted error strings. Fail closed for both
  configured-invalid PostgreSQL credential variables. SQLite metadata is queried
  only after matching CANTOPEN and a SQLite filename; PostgreSQL auto-init still
  never runs and retains `create-pg-db` guidance.

- [x] **Step 6: Add real backend integration proofs and verify**

  Exercise missing SQLite (auto-init succeeds) and missing PostgreSQL database
  (serve/open fails; no initialization attempt). Then run:

  ```bash
  devtool run -- cargo nextest run -p jaunder mailer
  devtool run -- cargo nextest run -p jaunder commands
  devtool run -- cargo nextest run -p storage postgres_password_from_env
  devtool run -- cargo xtask check
  ```

  Expected: all thirteen sources/classifications pass, absent SMTP alone selects
  Noop, and backend startup/credential classification remains exact.

- [x] **Step 7: Commit**

  ```bash
  git add server/src/mailer/factory.rs server/src/mailer/smtp.rs server/src/commands.rs storage/src/postgres/open.rs server/tests/misc/commands.rs server/tests/misc/postgres/commands.rs server/tests/storage/mod.rs
  git commit -m "fix(server): preserve startup and SMTP failure sources"
  ```

### Task 5: AtomPub, Media, and Projector Request Boundaries

**Files:**

- Modify: `Cargo.lock`
- Modify: `common/src/atompub/error.rs`
- Modify: `common/src/atompub/entry.rs`
- Modify: `host/src/error.rs`
- Modify: `server/src/atompub/guards.rs`
- Modify: `server/src/atompub/error.rs`
- Modify: `server/src/atompub/media.rs`
- Modify: `server/src/atompub/mod.rs`
- Modify: `server/src/atompub/posts.rs`
- Modify: `server/src/feed/handlers.rs`
- Modify: `server/src/media.rs`
- Modify: `server/src/projector/document.rs`
- Modify: `server/src/projector/handlers.rs`
- Modify: `storage/src/media_manager.rs`
- Modify: `web/Cargo.toml`
- Modify: `web/src/media/api.rs`
- Test: `server/tests/atompub/atompub_service.rs`
- Test: `server/tests/feed/feed_handlers.rs`
- Test: `server/tests/misc/media_handlers.rs`
- Test: `server/tests/projector/permalink.rs`
- Test: `server/tests/projector/listing.rs`
- Test: `server/tests/projector/tags.rs`
- Test: in-file unit tests in the modified common, storage, and web modules

**Interfaces:**

- `AtomPubError` has distinct source-carrying writer and UTF-8 variants.
  `base_url` returns `Result<Option<BaseUrl>, sqlx::Error>`; `required_base_url`
  maps `Ok(None)` only to `BaseUrlRequired` and retains an `Err(sqlx::Error)` in
  a source-carrying `HandlerError::Internal` variant.
- `MediaManager::first_file_in_dir` returns
  `Result<Option<PathBuf>, io::Error>`; both initial `read_dir` and later
  `next_entry` errors propagate before an upload result exists.
- A missing media `Extension<PathBuf>` is a server composition invariant:
  preserve the extractor rejection as an internal source. Multipart errors are
  classified exhaustively: malformed client multipart is validation, while
  `StreamReadFailed`, `LockFailure`, and equivalent infrastructure/invariant
  variants retain their source as internal failures. Upload mapping takes error
  ownership. The pure media-open classifier maps only `io::ErrorKind::NotFound`
  to 404 and preserves every other `io::Error` as a 500 boundary failure.
- `permalink_response` and `timeline_response` keep 500 and call
  `InternalError::emit_boundary_failure()` once. `tag_response` keeps its
  no-store CSR-shell 200 and reports once. A profile storage failure likewise
  reports once as `server.projector.profile` before preserving the no-store HTTP
  200 CSR shell.
- Feed regeneration and cache-read failures retain their typed source, return a
  sanitized 500 body, and emit exactly one boundary event/metric; source text is
  never copied into the client response.
- `PageSeed` serialization remains classified as structurally infallible: every
  closed variant uses only derived string/integer/sequence/newtype serializers,
  with no map key or custom serializer. An exhaustive no-wildcard variant test
  must serialize representative values for every variant and preserve the
  defensive `null` fallback classification; no fake warning path is added.
- This task owns all eleven Task 5 amendments: AtomPub writer/UTF-8 and
  `base_url`; feed regeneration/cache boundaries; profile degradation; dedup
  `read_dir`/`next_entry`; and all three media extract/multipart/upload
  mappings.

- [x] **Step 1: Add AtomPub propagation red tests**

  Call the private serialization helper with `Err(atom_syndication::Error)` and
  invalid UTF-8 bytes separately; assert `AtomPubError::source` downcasts to the
  exact writer or `FromUtf8Error` source. Inject a failing identity store
  through `required_base_url`; assert the source remains typed and the AtomPub
  request fails, while a successful `Ok(None)` still selects the documented
  unconfigured-base-url response.

- [x] **Step 2: Add media propagation and classification red tests**

  Give the dedup probe a narrow private directory-reader adapter. Inject initial
  `read_dir` failure and a `next_entry` failure after one entry separately;
  assert upload returns the exact I/O source and never reports success. Inject
  missing `Extension<PathBuf>` and assert an internal error retaining the
  extractor rejection. Construct every multer error class: malformed client
  multipart maps to validation; `StreamReadFailed`, `LockFailure`, and each
  infrastructure/invariant variant map internal and retain the exact source.
  Inject `MediaManager::upload` I/O/storage failure and assert ownership reaches
  a source-carrying `InternalError`.

  Use constructed `io::ErrorKind::NotFound` and `PermissionDenied` open errors;
  assert NotFound maps to 404 while PermissionDenied remains downcastable, maps
  to 500, and emits one boundary event.

- [x] **Step 3: Add projector, feed, and PageSeed proofs**

  Extend projector tests with captured events/metrics for permalink, timeline,
  and tag error arms. Mock `fetch_user_posts`/storage failure in `profile`;
  assert the exact no-store CSR-shell status/body remains HTTP 200 and exactly
  one swallowed event/metric has context `server.projector.profile`.

  Inject feed regeneration and feed-cache read failures separately; assert each
  concrete source reaches the boundary carrier, the response is a sanitized 500
  without source text, and exactly one boundary event/metric is emitted.

  Add an exhaustive `match` over every `PageSeed` variant with no wildcard,
  construct and serialize one representative of each, and assert none selects
  the defensive `null` fallback. Document why derived closed fields make
  serialization structurally infallible; this is the required proof, not an
  injectable impossible failure.

- [x] **Step 4: Run and observe every missing contract**

  ```bash
  devtool run -- cargo nextest run -p common atompub
  devtool run -- cargo nextest run -p storage media_manager
  devtool run -- cargo nextest run -p web --features server media
  devtool run -- cargo nextest run -p jaunder atompub
  devtool run -- cargo nextest run -p jaunder media
  devtool run -- cargo nextest run -p jaunder projector
  devtool run -- cargo nextest run -p jaunder feed_handlers
  ```

  Expected before implementation: the eleven amendment proofs fail because
  sources are string-erased, exposed, or swallowed; projector reporting is
  incomplete; non-NotFound media I/O is misclassified.

  Red-phase result: common, storage, and server failed on the missing typed
  variants and classifier seams. The original web command returned green without
  compiling its `feature = "server"` tests; the corrected command above now
  exercises that contract explicitly.

- [x] **Step 5: Implement the owning-domain mappings**

  Add typed AtomPub variants and propagate the identity-store result. Return
  `Result` from the dedup probe and use `?` for both enumeration operations.
  Classify each owned media/multipart source semantically. Match media-open
  `io::ErrorKind`, not platform permissions. Project feed failures through the
  source-carrying boundary with sanitized 500s. Report only the tag/profile
  intentional CSR continuations; raw-Axum/feed 500 paths emit boundary
  diagnostics. Never report propagated dedup, extractor, multipart, upload,
  feed, or AtomPub failures as swallowed.

- [x] **Step 6: Add deterministic disappearance HTTP coverage**

  Seed a media record and file, build the router, remove the file after lookup
  setup, and assert 404. Do not depend on chmod/root behavior. Preserve existing
  cache and range behavior.

- [x] **Step 7: Verify all request-boundary contracts**

  ```bash
  devtool run -- cargo nextest run -p common atompub
  devtool run -- cargo nextest run -p storage media_manager
  devtool run -- cargo nextest run -p web --features server media
  devtool run -- cargo nextest run -p jaunder atompub
  devtool run -- cargo nextest run -p jaunder media
  devtool run -- cargo nextest run -p jaunder projector
  devtool run -- cargo nextest run -p jaunder feed_handlers
  devtool run -- cargo xtask check
  ```

  Expected: all eleven amendments have passing source/report proofs; the
  PageSeed exhaustive proof passes; public status/body/cache/range behavior is
  unchanged except the required sanitized/non-NotFound 500 corrections.

- [x] **Step 8: Commit**

  ```bash
  git add Cargo.lock common/src/atompub/error.rs common/src/atompub/entry.rs docs/superpowers/plans/2026-08-13-issue-58-error-swallowing-audit.md host/src/error.rs server/src/atompub/error.rs server/src/atompub/guards.rs server/src/atompub/media.rs server/src/atompub/mod.rs server/src/atompub/posts.rs server/src/feed/handlers.rs server/src/media.rs server/src/projector/document.rs server/src/projector/handlers.rs storage/src/media_manager.rs web/Cargo.toml web/src/media/api.rs server/tests/atompub/atompub_service.rs server/tests/feed/feed_handlers.rs server/tests/misc/media_handlers.rs server/tests/projector/permalink.rs server/tests/projector/listing.rs server/tests/projector/tags.rs
  git commit -m "fix(server): preserve request-boundary failure sources"
  ```

### Task 6: Native Runtime Propagation and Continuation Reporting

**Files:**

- Modify: `host/src/capture.rs`
- Modify: `server/src/backup.rs`
- Modify: `server/src/feed/worker.rs`
- Modify: `server/src/runtime_file.rs`
- Modify: `server/src/websub/contract.rs`
- Modify: `server/src/websub/file_capture.rs`
- Modify: `server/src/websub/http.rs`
- Modify: `storage/src/backup.rs`
- Modify: `storage/src/feed_events.rs`
- Modify: `storage/src/helpers.rs`
- Modify: `storage/src/media_manager.rs`
- Modify: `storage/src/postgres/atomic.rs`
- Modify: `storage/src/postgres/backup.rs`
- Modify: `storage/src/postgres/feed_events.rs`
- Modify: `storage/src/postgres/posts.rs`
- Modify: `storage/src/posts.rs`
- Modify: `storage/src/sqlite/atomic.rs`
- Modify: `storage/src/sqlite/backup.rs`
- Modify: `storage/src/sqlite/feed_events.rs`
- Modify: `storage/src/sqlite/posts.rs`
- Modify: `storage/src/users.rs`
- Modify: `web/src/audiences/api.rs`
- Test: in-file unit tests in every modified storage/web module
- Test: `server/tests/feed/feed_worker.rs`
- Test: `server/tests/feed/feed_events_hook.rs`

**Interfaces:**

- `WebSubError::Http` retains a boxed typed I/O or HTTP source. File-capture and
  HTTP publish errors propagate to the worker; they are not swallowed reports.
- Every remaining unexpected native continuation calls
  `host::error::report_swallowed` exactly once at the useful aggregation
  boundary. Existing tracing-only warnings at corrupt-row purge/feed-cache
  decode sites are replaced by the atomic reporter, not duplicated. Every call
  uses the baseline inventory's static context verbatim; the
  delivered-remediation ledger records that exact callsite and test.
- Capture-directory creation, scheduled status/ack/rollback, backup measurement,
  cleanup, transaction rollback, corrupt-row quarantine, label degradation, and
  runtime-file removal preserve the named primary result. Recursive/population
  failures aggregate once rather than warning per lexical hit or directory
  entry.
- This task owns all 27 Task 6 amendments: the two propagated WebSub mappings
  and these 25 continued rows: invalid capture-directory configuration;
  temporary-backup cleanup; corrupt claimed-feed decode; dummy-password hash
  construction/fallback (two lexical rows, one report); quota-temp cleanup;
  dedup-temp cleanup; PostgreSQL password-reset, backup export/restore,
  post-tag, and post-update rollbacks; PostgreSQL corrupt purge; feed-cache path
  decode; SQLite password-reset, invite, backup export/restore/foreign-key,
  post-tag, and post-update rollbacks; SQLite corrupt purge; dummy password
  verification; and both subscriber-label lexical rows as one report.

- [ ] **Step 1: Add propagated WebSub source tests**

  Inject file write/flush and HTTP transport failure separately through the
  existing clients. Assert `WebSubError::source` downcasts to the concrete I/O
  or HTTP error before worker retry handling, and that the worker receives an
  error rather than a successful ping. These are the red/green proofs for the
  two propagated Task 6 amendments.

- [ ] **Step 2: Add filesystem and decode continuation seams**

  Use small private operation parameters at the owning functions—no global fault
  injector and no `cfg(test)` production branch—to inject:
  - temporary backup `remove_dir_all`, quota-temp `remove_file`, and both
    dedup-temp unlink sites; assert enclosing success/domain error is unchanged
    and each owning operation reports once;
  - capture-directory creation, backup-size metadata/read-dir/entry recursion,
    and runtime-file removal; assert the existing fallback/primary result,
    exactly one aggregate report, and no report for `NotFound`;
  - invalid-Unicode `JAUNDER_CAPTURE_DIR`; assert capture is disabled, startup
    is unchanged, and one redacted `host.capture.directory_config` report;
  - corrupt `ClaimedRow` and feed-cache `FeedPath` decode; tamper one row beside
    a later valid row, assert continuation, redaction, and one report;
  - injected dummy Argon2 hash failure through a fresh private OnceLock helper;
    assert the timing-safe fallback hash, unchanged invalid-user behavior, and
    one `storage.auth.dummy_password_hash` report across both lexical rows; and
  - dummy password verification failure; assert `InvalidCredentials` remains
    primary and capture one report.

- [ ] **Step 3: Add transaction-secondary failure seams**

  Extract narrow private transaction-finish helpers that accept the already
  determined primary result and the rollback/foreign-key-reset result.
  Production passes the real sqlx result; tests inject the secondary error and
  assert the exact primary variant/value is returned plus one report. Cover
  separately:
  - PostgreSQL password-reset token rejection, backup export, backup restore,
    post-tag `PostNotFound`, and post-update `NotFound` and `Unauthorized`;
  - SQLite password-reset token rejection, invite registration, backup export,
    backup restore rollback, backup restore foreign-key re-enable, post-tag, and
    post-update.

  The post-update PostgreSQL test exercises `NotFound` and `Unauthorized`
  separately. Backup tests inject body and secondary failures together and
  compare the returned primary error before/after by variant and source.

- [ ] **Step 4: Add backend-parametric quarantine and web degradation tests**

  Use `#[apply(backends)]` plus `Backend::setup()` for corrupt claimed-feed
  decode/purge, password-reset rollback result preservation, and post update/tag
  domain-result preservation on both databases. Force corrupt-row DELETE failure
  after valid/corrupt partitioning and assert the valid batch returns with one
  report. Use paired dialect-specific tests only for backup transaction
  mechanics, sharing the same primary-result assertion helper.

  In `list_my_subscribers`, inject one `get_user` failure and assert the raw
  subscriber reference label is returned; the `.ok()` and `Err(_)` lexical rows
  together produce one `web.audiences.subscriber_label_lookup` report.

- [ ] **Step 5: Retain the pre-existing host/server continuation proofs**

  Keep dedicated tests for capture creation; scheduled feed status,
  ack/rollback, ping, and regeneration continuations; backup measurement and
  pruning; and runtime-file removal. Each test asserts the exact primary return
  and one report at its existing static context. Expected NotFound and domain
  parse mismatches remain unreported.

- [ ] **Step 6: Run and observe current omissions**

  ```bash
  devtool run -- cargo nextest run -p host capture
  devtool run -- cargo nextest run -p storage continuation_reporting
  devtool run -- cargo nextest run -p web continuation_reporting
  devtool run -- cargo nextest run -p jaunder websub
  devtool run -- cargo nextest run -p jaunder feed
  devtool run -- cargo nextest run -p jaunder backup
  devtool run -- cargo nextest run -p jaunder runtime_file
  ```

  Expected before implementation: WebSub downcasts and each of the 25 new
  continuation report assertions fail; primary-result assertions document the
  behavior that implementation must preserve.

- [ ] **Step 7: Implement typed propagation and one report per continuation**

  Move WebSub sources into source-carrying variants. At continuations preserve
  the typed source whenever PII-safe; otherwise use `SwallowedSource::Redacted`.
  Add why-comments at the continuation, not mechanics comments. Remove replaced
  `tracing::warn!` calls so one operation cannot double-report. Do not report
  propagated WebSub errors, expected NotFound, or parse mismatches.

- [ ] **Step 8: Verify all 27 amendments and primary-result invariants**

  ```bash
  devtool run -- cargo nextest run -p host capture
  devtool run -- cargo nextest run -p storage continuation_reporting
  devtool run -- cargo nextest run -p web continuation_reporting
  devtool run -- cargo nextest run -p jaunder websub
  devtool run -- cargo nextest run -p jaunder feed
  devtool run -- cargo nextest run -p jaunder backup
  devtool run -- cargo nextest run -p jaunder runtime_file
  devtool run -- cargo xtask check
  ```

  Expected: both WebSub sources propagate; every one of the 25 continued rows
  reports once; backend-parametric cases pass for SQLite and PostgreSQL; every
  asserted primary result is unchanged.

- [ ] **Step 9: Commit**

  ```bash
  git add host/src/capture.rs server/src/backup.rs server/src/feed/worker.rs server/src/runtime_file.rs server/src/websub/contract.rs server/src/websub/file_capture.rs server/src/websub/http.rs server/tests/feed/feed_worker.rs server/tests/feed/feed_events_hook.rs storage/src/backup.rs storage/src/feed_events.rs storage/src/helpers.rs storage/src/media_manager.rs storage/src/postgres/atomic.rs storage/src/postgres/backup.rs storage/src/postgres/feed_events.rs storage/src/postgres/posts.rs storage/src/posts.rs storage/src/sqlite/atomic.rs storage/src/sqlite/backup.rs storage/src/sqlite/feed_events.rs storage/src/sqlite/posts.rs storage/src/users.rs web/src/audiences/api.rs
  git commit -m "fix(runtime): preserve and report native failures"
  ```

### Task 7: Bounded Client Reporter

**Files:**

- Create: `common/src/client_telemetry.rs`
- Modify: `common/src/lib.rs`
- Create: `client/src/telemetry.rs`
- Modify: `client/src/lib.rs`
- Modify: `client/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**

- Produces closed, serde-backed wire types in `common::client_telemetry`:

  ```rust
  pub const CLIENT_TELEMETRY_VERSION: u8 = 1;
  pub enum ClientErrorKind { Network, Storage, Decode, Dialog, FormData, Internal }
  pub enum ClientErrorContext {
      ThemeStorageRead,
      ThemeStorageWrite,
      SessionMarkerRead,
      SessionMarkerWrite,
      SessionMarkerRemove,
      ProjectorSeedDecode,
      PublishConfirm,
      DeleteConfirm,
      MediaFormData,
  }
  pub enum ClientSourceKind {
      StorageUnavailable,
      StorageOperation,
      InvalidSeed,
      DialogUnavailable,
      FormDataCreate,
      FormDataAppend,
  }
  pub struct ClientTelemetryEvent {
      pub version: u8,
      pub kind: ClientErrorKind,
      pub context: ClientErrorContext,
      pub source_kind: ClientSourceKind,
  }
  ```

  These are the complete audited variants; none carries data. Serde uses stable
  snake_case tokens and denies unknown fields.

- `client::telemetry::Reporter<T>` is host-testable with an injected transport.
  Its synchronous `report_swallowed(kind, context, source_kind) -> ()` logs
  first, starts at most one send, drops a concurrent report, and clears
  in-flight only through the transport completion callback.
- The wasm adapter sends `POST /api/client-telemetry`, JSON content type,
  `credentials: include`, `keepalive: true`; it never calls the reporter from
  its own failure path.

- [ ] **Step 1: Add wire-shape tests**

  Serialize every enum variant; assert snake_case stable tokens, no dynamic
  field, exact version, `deny_unknown_fields`, and unknown version/enum
  rejection on decode. Assert a maximally encoded valid event is below 1,024
  bytes.

- [ ] **Step 2: Add host reporter state-machine tests**

  With a capturing console sink and manually-completed fake transport, assert:
  local warning precedes send; return is synchronous `()`; first report is sent;
  second while in flight is logged and dropped; success, auth rejection, 429,
  and network completion each clear the slot; no completion path invokes
  transport twice or reports recursively.

- [ ] **Step 3: Run and observe compile failure**

  ```bash
  devtool run -- cargo nextest run -p common client_telemetry
  devtool run -- cargo nextest run -p client telemetry
  ```

- [ ] **Step 4: Implement the deep interface and wasm adapter**

  Keep reporter state behind a single module-owned instance for browser callers.
  Add only the precise `web-sys` features needed for Request/RequestInit/Headers
  and `Window::fetch`; do not add an OTel dependency or JS bundler.

- [ ] **Step 5: Verify wasm compilation, size/static policy, and commit**

  ```bash
  devtool run -- cargo nextest run -p common client_telemetry
  devtool run -- cargo nextest run -p client telemetry
  devtool run -- cargo xtask check
  git add common/src/client_telemetry.rs common/src/lib.rs client/src/telemetry.rs client/src/lib.rs client/Cargo.toml Cargo.lock
  git commit -m "feat(client): add bounded swallowed error reporter"
  ```

### Task 8: Authenticated Client-Telemetry Intake

**Files:**

- Create: `server/src/client_telemetry.rs`
- Modify: `server/src/lib.rs`
- Modify: `host/src/auth.rs`
- Modify: `host/src/error.rs`
- Create: `server/tests/misc/client_telemetry.rs`
- Modify: `server/tests/misc/mod.rs`
- Modify: `server/tests/helpers/http.rs`

**Interfaces:**

- Produces `POST /api/client-telemetry` with only
  `Extension<Arc<dyn SessionStorage>>`,
  `Extension<Arc<ClientTelemetryLimiter>>`, and a dedicated browser-session
  extractor; it never receives `AppState`.
- Produces a pure cookie-only parser. It ignores Authorization entirely and does
  not call `host::auth::resolve_credential`.
- Produces
  `host::auth::resolve_session_cookie(headers: &http::HeaderMap) -> Option<RawToken>`;
  `resolve_credential` delegates its cookie branch to the same parser while
  retaining cookie-over-Authorization precedence.
- Produces an injected-clock limiter with constants `BURST=5`, `REFILL=1/min`,
  `STALE=15min`, `MAX_CLEANUP=64`, a map keyed by `UserId`, and a round-robin
  ring containing exactly one entry per map bucket.
- Produces
  `host::error::report_client_swallowed(kind: ErrorKind, class: ErrorClass, context: &'static str, source_kind: ClientSourceKind)`.
  It emits the bounded fixed WARN and calls the private metric helper with
  `Swallowed/Client` exactly once.

- [ ] **Step 1: Add pure limiter tests**

  With manual time, assert: five immediate accepts; sixth reject; one token
  after one minute; separate users independent; a full bucket retained before 15
  idle minutes and removed after; duplicate attempts do not duplicate ring
  entries; every retained entry is eventually visited; one cleanup visits at
  most 64; a new limiter has zero buckets/ring entries and no inherited token
  state.

- [ ] **Step 2: Add backend-parametric HTTP rejection tests**

  Through a fresh `Backend::setup()` router assert exact status codes: missing,
  malformed, expired, and unknown session cookie => 401; valid Bearer and valid
  Basic/app password without cookie => 401; malformed JSON, unsupported version,
  unknown enum => 400; missing/text content type => 415; 1,024-byte
  authenticated body reaches decode while 1,025 => 413; closed session storage
  => 500; sixth accepted-user event => 429. Assert every rejection emits no
  intake warning, `jaunder.errors`, or `session_validation` metric.

- [ ] **Step 3: Add backend-parametric acceptance tests**

  A valid cookie event returns 204 and emits one fixed WARN plus exactly one
  `jaunder.errors{disposition=swallowed,origin=client}`. With the same valid
  cookie plus Bearer/Basic headers, cookie authentication wins. Assert the
  route's test constructor supplies only session storage and limiter extensions.

- [ ] **Step 4: Run and observe route absence**

  ```bash
  devtool run -- cargo nextest run -p jaunder client_telemetry
  ```

  Expected before implementation: FAIL/404.

- [ ] **Step 5: Implement parsing, guard, limiter, handler, and composition**

  Apply the body limit before unbounded buffering. Authenticate before emitting
  any diagnostic. The dedicated guard maps storage internal errors to 500
  without `session_validation`. Rate-limit rejection is a silent generic 429.
  Accepted closed enums map exhaustively to bounded host
  `ErrorKind`/`ErrorClass` and call `host::error::report_client_swallowed`; the
  handler cannot call the private metric helper directly.

- [ ] **Step 6: Verify both backends and commit**

  ```bash
  devtool run -- cargo nextest run -p jaunder client_telemetry
  devtool run -- cargo xtask check
  git add host/src/auth.rs host/src/error.rs server/src/client_telemetry.rs server/src/lib.rs server/tests
  git commit -m "feat(server): ingest bounded client diagnostics"
  ```

### Task 9: Audited Browser Callers and End-to-End Delivery

**Files:**

- Modify: `client/src/storage.rs`
- Modify: `client/src/dialog.rs`
- Modify: `client/src/upload.rs`
- Modify: `csr/src/lib.rs`
- Modify: `web/src/app/component.rs`
- Modify: `web/src/auth/marker_storage.rs`
- Modify: `web/src/posts/component.rs`
- Modify: `web/src/media/component.rs`
- Create: `end2end/tests/client-telemetry.spec.ts`
- Modify: `end2end/tests/capture-trace.ts`

**Interfaces:**

- Each unexpected browser API failure that intentionally preserves caller state
  invokes `client::telemetry::report_swallowed` exactly once with closed enums.
- Expected parse/downcast/absence remains ordinary control flow and emits
  nothing.
- Storage primitives remain truthful `Result`s; caller-specific policy and
  static context stay at the caller.
- Dialog and FormData primitives distinguish cancellation/absence from thrown
  API failure so callers can report only the latter.

- [ ] **Step 1: Add pure/host classification tests**

  Pin the mapping from `StorageError`, seed decode failure, dialog throw,
  FormData construction/append failure, and expected no-file/cancel paths to
  either one closed event or no event. Keep arbitrary browser exception text
  local-only.

- [ ] **Step 2: Refine browser primitive return types**

  Change `confirm` and upload helpers only enough to separate expected
  user/no-file outcomes from browser exceptions. Migrate every caller in the
  same change; no compatibility aliases or silent `Option` shims remain.

- [ ] **Step 3: Wire all audited continued callers**

  Report failed theme/session-marker access, projector seed JSON decode, dialog
  exception, and FormData creation/append before preserving the existing visible
  behavior. Do not report missing DOM nodes, absent localStorage values, user
  cancellation, parse mismatch used as feature detection, or failures already
  rendered explicitly.

- [ ] **Step 4: Add a real browser-to-server proof**

  Use existing Playwright fault injection to force one audited browser operation
  to fail while authenticated. Assert the console warning occurs before the
  keepalive request starts, the caller's user-visible state is unchanged, and
  the captured server warning/metric has `swallowed/client`. Close the page
  after the request starts; do not require delivery after termination.

- [ ] **Step 5: Verify and commit**

  ```bash
  devtool run -- cargo nextest run -p client telemetry
  devtool run -- cargo xtask e2e-local client-telemetry.spec.ts
  devtool run -- cargo xtask check
  git add client/src csr/src web/src end2end
  git commit -m "fix(client): report swallowed browser failures"
  ```

### Task 10: Explicit Profile and Audience Failure State

**Files:**

- Modify: `web/src/profile/component.rs`
- Create: `web/src/profile/page_state.rs`
- Modify: `web/src/profile/mod.rs`
- Modify: `web/src/posts/component.rs`
- Modify: `web/src/posts/page_state.rs`
- Modify: `end2end/tests/profile.spec.ts`
- Modify: `end2end/tests/posts.spec.ts`

**Interfaces:**

- Profile default-format state has distinct Loading, Ready(PostFormat), and
  Failed outcomes. Only Ready enables Save; failure never writes a fabricated
  Markdown preference.
- Named-audience state has distinct Loading, Ready(Vec<Summary>)—including true
  loaded-empty—and Failed. Publish/update remains gated until Ready; Failed
  never submits an empty selection as if it were real data.

- [ ] **Step 1: Add host-compiled decision tests**

  Assert profile Loading/Failed cannot dispatch and Ready dispatches the fetched
  format. Assert audience Loading/Failed cannot submit, Ready(empty) can render
  a genuine empty state and submit according to existing selection rules, and
  Ready(non-empty) preserves named selection.

- [ ] **Step 2: Run and observe fabricated defaults**

  ```bash
  devtool run -- cargo nextest run -p web default_post_format
  devtool run -- cargo nextest run -p web audience_picker
  ```

  Expected before implementation: FAIL because `unwrap_or(Markdown)` and
  `Result::ok().unwrap_or_default()` erase the failures.

- [ ] **Step 3: Render explicit states and action gates**

  Preserve existing successful copy/layout. On failure render a stable `.error`
  node and disable or omit the affected action. Use a distinct loaded-empty
  message for no named audiences. Do not emit client-swallow telemetry: these
  server failures already returned through the server boundary and are visible.

- [ ] **Step 4: Add Playwright failure flows**

  In profile, force `get_default_post_format` failure; assert explicit error, no
  fabricated selected Markdown save, and disabled/absent Save. In composer,
  force `list_mine` failure; assert explicit error, no empty picker masquerade,
  and publish remains gated. Retain success coverage for loaded-empty and
  populated audiences.

- [ ] **Step 5: Verify and commit**

  ```bash
  devtool run -- cargo nextest run -p web default_post_format audience_picker
  devtool run -- cargo xtask e2e-local profile.spec.ts
  devtool run -- cargo xtask e2e-local posts.spec.ts
  devtool run -- cargo xtask check
  git add web/src/profile web/src/posts end2end/tests/profile.spec.ts end2end/tests/posts.spec.ts
  git commit -m "fix(web): expose failed preference and audience loads"
  ```

### Task 11: Developer-Tool Failure Visibility

**Files:**

- Modify: `server/build.rs`
- Test: `server/tests/build_script.rs`
- Modify: `xtask/src/adr.rs`
- Modify: `xtask/src/adr_readme.rs`
- Modify: `xtask/src/coverage/run.rs`
- Modify: `xtask/src/coverage/crap.rs`
- Modify: `xtask/src/coverage/probe.rs`
- Modify: `xtask/src/pr/gh.rs`
- Modify: `xtask/src/traces/analyze.rs`
- Modify: `xtask/src/traces/boot_phases.rs`
- Modify: `xtask/src/traces/parse.rs`
- Modify: `xtask/src/steps/e2e_local.rs`
- Modify: `xtask/src/steps/no_full_reload_check.rs`
- Modify: `xtask/src/steps/proffered_filename_check.rs`
- Modify: `xtask/src/steps/proffered_secret_check.rs`
- Modify: `xtask/src/steps/sequence_check.rs`
- Modify: `xtask/src/steps/sqlx_newtype_bind_check.rs`
- Modify: `xtask/src/steps/test_pattern_check.rs`
- Modify: `xtask/src/steps/nix.rs`
- Modify: `tools/devtool/src/pg.rs`
- Modify: `tools/devtool/src/run.rs`

**Interfaces:**

- Correctness/population functions return `Result`/failed `StepResult` with path
  and source; unreadable entries cannot become an empty or smaller green
  population. Warning and continuation are forbidden for these failures.
- Best-effort diagnostic/cleanup helpers preserve the primary result and print
  one fixed warning to stderr identifying ignored disposition and static
  context. Existing stdout/JSON contracts remain byte-parseable.
- The sixteen Task 11 amendments split exactly: fail closed for build-script
  staging cleanup, `adr_readme::adr_files` file-type I/O, both
  `sequence_check::filenames` read-dir/file-type paths, malformed present
  trace-JSON attributes, and genuinely absent e2e `PATH` while preserving
  non-Unicode `PATH` as `OsString`; warn-only for invalid-Unicode
  `JAUNDER_E2E_WORKERS`, both `rate_limit_reset` probe failures, the three
  coverage-status rows, the three doctest-status rows, and `nix::eval_out_path`.

- [ ] **Step 1: Add exact fail-closed correctness tests**

  In `server/tests/build_script.rs`, route staging cleanup through an injected
  remover, force `remove_dir_all` failure, and assert the build aborts before
  directory creation or asset copy with the staging path and typed I/O source.
  In `adr_readme.rs`, inject `DirEntry::file_type` failure and assert the parity
  step fails with entry path and I/O source; it must not skip the entry. In
  `sequence_check.rs`, inject missing/unreadable directory and later `file_type`
  failure separately for both ADR and migration inputs; assert path/source
  failure instead of an empty or smaller population. Inject non-Unicode e2e
  `PATH` and assert byte-for-byte `OsString` preservation; inject absent `PATH`
  and assert contextual `VarError`, never empty-path search.

  In `traces/parse.rs`, inject an absent JSON attribute and malformed/valid
  present attributes. Absence remains `Ok(None)`; malformed JSON retains
  `serde_json::Error` plus span source and attribute key. Drive the malformed
  value through both `traces analyze` and boot-phase call paths and assert a
  failed `StepResult`, not an empty section. These tests are mandatory for all
  six fail-closed amendment rows—there is no stderr-warning alternative.

  Retain equivalent fail-closed tests for ADR filename/draft enumeration;
  coverage exemption and CRAP allow-marker reads; and source populations in
  `no_full_reload_check`, `proffered_filename_check`, `proffered_secret_check`,
  `sqlx_newtype_bind_check`, and `test_pattern_check`.

- [ ] **Step 2: Add exact warning-only amendment tests**

  Inject `rate_limit_reset` spawn failure and nonzero/malformed classification
  separately; assert the original `RateLimited` error is unchanged and each
  attempt writes exactly one `xtask.pr.rate_limit_reset` stderr warning.

  Inject invalid-Unicode `JAUNDER_E2E_WORKERS`; assert workers remains `1`, the
  e2e primary result/JSON stdout is unchanged, and exactly one redacted
  `xtask.e2e.workers_config` stderr warning is written.

  With an already-failed coverage gate, inject missing/unreadable `status.json`
  and syntactically malformed status separately. Assert the generic failed
  `StepResult` and JSON/stdout are byte-for-byte unchanged, and each attempt
  writes one `xtask.nix.coverage_status` warning. Repeat the two injected cases
  for an already-failed doctest gate and `xtask.nix.doctest_status`. Inject
  `nix eval` failure only after an e2e Nix build has failed; assert the failed
  build result/JSON is unchanged and one `xtask.nix.e2e_out_path` warning
  appears.

  These tests cover all ten warning-only amendment rows: lexical
  read/parse/fallback hits for coverage and doctests aggregate to one warning
  per failed status attempt, not three warnings.

- [ ] **Step 3: Add the remaining ancillary warning tests**

  Inject failures for coverage diagnostic dump creation/write, e2e diagnostic
  remove/copy/permission/rescue, probe-worktree RAII cleanup, ephemeral
  PostgreSQL stop/remove/join/emulated-signal cleanup, `devtool run` history
  prune, and timeout kill cleanup. For the probe, inject a failing worktree
  remover and assert the original probe verdict plus one fixed stderr warning.
  For every case, capture stderr and assert one contextual warning while the
  original success/failure result and JSON stdout are unchanged.

- [ ] **Step 4: Run and observe current omissions**

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml fail_closed_population
  devtool run -- cargo nextest run -p jaunder build_script_staging
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml trace_json_attr
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml ancillary_warning
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml e2e_env
  devtool run -- cargo nextest run --manifest-path tools/devtool/Cargo.toml
  ```

  Expected before implementation: build staging, population/PATH, and malformed
  trace-attribute tests demonstrate ignored, false-green, empty-path, or
  empty-section behavior; the ten exact amendment warnings and existing
  ancillary warnings are absent.

- [ ] **Step 5: Propagate correctness failures**

  Make staging removal return a contextual build error and abort before any
  asset copy. Convert `adr_filenames`, `draft_slugs`, `adr_files`, and
  `sequence_check::filenames` to `Result<Vec<_>>`; use `collect::<Result<_>>()`
  rather than flatten/filter on entry I/O. Make `parse_json_attr` return
  `Result<Option<Value>>`; migrate all analyzer and boot-phase callers so
  present malformed JSON reaches the owning failed `StepResult`, while absent
  attributes remain optional. Preserve e2e `PATH` as `OsString` and propagate
  `NotPresent` with context. Apply the same rule to every correctness population
  listed in Step 1. Do not add a syntax scanner, allowlist, or warning fallback.

- [ ] **Step 6: Warn for legitimate ancillary continuation**

  Use fixed `eprintln!` prefixes such as
  `xtask: warning: ignored failure while <static context>: {error}` and
  `devtool: warning: ...`. Add the redacted workers-config warning. Aggregate
  lexical rows, recursive copy, or cleanup failures at one useful boundary.
  Never overwrite the child/primary exit code or write to stdout/JSON.

- [ ] **Step 7: Verify tool contracts**

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml
  devtool run -- cargo nextest run --manifest-path tools/devtool/Cargo.toml
  devtool run -- cargo xtask check
  ```

  Expected: all six fail-closed amendments preserve path/source; all ten
  warning-only amendments preserve the primary result and warn once; remaining
  ancillary contracts pass without stdout/JSON drift.

- [ ] **Step 8: Commit**

  ```bash
  git add server/build.rs server/tests/build_script.rs xtask/src/adr.rs xtask/src/adr_readme.rs xtask/src/coverage/run.rs xtask/src/coverage/crap.rs xtask/src/coverage/probe.rs xtask/src/pr/gh.rs xtask/src/steps/e2e_local.rs xtask/src/steps/no_full_reload_check.rs xtask/src/steps/proffered_filename_check.rs xtask/src/steps/proffered_secret_check.rs xtask/src/steps/sequence_check.rs xtask/src/steps/sqlx_newtype_bind_check.rs xtask/src/steps/test_pattern_check.rs xtask/src/steps/nix.rs xtask/src/traces/analyze.rs xtask/src/traces/boot_phases.rs xtask/src/traces/parse.rs tools/devtool/src/pg.rs tools/devtool/src/run.rs
  git commit -m "fix(tools): expose ignored correctness and cleanup failures"
  ```

### Task 12: Mechanical Reconciliation and Delivery Evidence

**Files:**

- Create: `xtask/src/steps/error_swallowing_inventory_check.rs`
- Modify: `xtask/src/lib.rs`
- Modify:
  `docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-inventory.md`
- Modify: `docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-audit.md`
  only to append delivered AC evidence, not to change approved behavior
- Modify: this plan's checkboxes in real time

**Interfaces:**

- Produces a final inventory whose recipes reproduce exact final totals with
  zero missing/duplicate/unclassified hits and no silent unexpected failure.
- Adds a checked-in delivered-remediation ledger keyed by the 88 stable
  `(path, containing symbol, normalized expression)` triples copied from the
  pre-implementation inventory before its rows are reconciled. Each entry names
  final disposition, owning task, commit, exact behavioral test, command, and a
  final outcome (`row:<final key>` or `removed:<reason>`); continued outcomes
  also name reporting site and primary result.
- Adds `error-swallowing-inventory` to `cargo xtask check`. It parses only the
  checked-in inventory and ledger; it does not scan Rust syntax or create an
  `.ok()`/`map_err`/wildcard allowlist.
- Produces AC1-AC23 evidence, including explicit mappings for every inherited
  #898 and #899 acceptance item.

**Cold-review finding matrix:**

| `InventoryReview58` finding                       | Exact revised owner                                                                                                                                                                                           |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rebuild `map_err` reduction from every raw record | Task 12 Step 1 splits AtomPub/tagged-URL expressions, merges the third file-mail occurrence, adds email-status/media-upload records, and recomputes totals; Steps 2-3 reject a missing stable remediation key |
| Propagate dedup directory probe failures          | Task 5 Steps 2 and 5 cover initial `read_dir` and later `next_entry` independently                                                                                                                            |
| Remediate string-erased propagated sources        | Task 2 Steps 2/5 (password/account); Task 4 Steps 1-2/5 (CLI/SMTP); Task 5 Steps 1-2/5 (AtomPub/media); Task 6 Steps 1/7 (WebSub)                                                                             |
| Propagate AtomPub identity read                   | Task 5 Steps 1 and 5                                                                                                                                                                                          |
| Split observability continuations                 | Task 3 Step 3 cases 1-3; Task 12 Step 1 records distinct complete-expression keys                                                                                                                             |
| Prove shutdown and subscriber-install failures    | Task 3 Step 3 cases 4-6                                                                                                                                                                                       |
| Fail closed for population amendments             | Task 11 Steps 1 and 5; warning is forbidden                                                                                                                                                                   |
| Correct inverted dispositions and totals          | Task 2 Step 2 pins Argon2 mismatch versus failure; Task 12 Step 1 corrects all named classifications and recomputes totals                                                                                    |
| Prove PageSeed serialization decision             | Task 5 Step 3 exhaustively proves the structurally infallible closed representation                                                                                                                           |

**Amendment ownership matrix:**

| Inventory amendments                                                                             | Owning red/green step            | Implementation/verification      |  Count |
| ------------------------------------------------------------------------------------------------ | -------------------------------- | -------------------------------- | -----: |
| Password hash/verify mappings; account-command lines 219, 247, 255                               | Task 2 Step 2                    | Task 2 Steps 5-6, commit Step 7  |      7 |
| Exporter source mappings; diagnostic/configuration fallbacks; shutdowns; subscriber install      | Task 3 Step 3 cases 1-6 and 8-15 | Task 3 Steps 5-6, commit Step 7  |     14 |
| Nine command mappings; SMTP lines 65, 79/83; PostgreSQL password env/file                        | Task 4 Steps 1-3                 | Task 4 Steps 5-6, commit Step 7  |     13 |
| AtomPub writer/UTF-8/`base_url`; feed boundaries; profile; dedup directory; three media mappings | Task 5 Steps 1-3                 | Task 5 Steps 5-7, commit Step 8  |     11 |
| WebSub file/HTTP plus every 25 continuation row enumerated in Task 6 Interfaces                  | Task 6 Steps 1-5                 | Task 6 Steps 7-8, commit Step 9  |     27 |
| Build staging, ADR/sequence/PATH/trace-JSON fail-closed; workers/rate-limit/Nix warnings         | Task 11 Steps 1-3                | Task 11 Steps 5-7, commit Step 8 |     16 |
| **All stable remediation keys**                                                                  |                                  |                                  | **88** |

- [ ] **Step 1: Rerun every inventory recipe against the final tree**

  Before changing classified rows, copy the exact 88 marked baseline keys into
  the delivered-remediation ledger so expression removal cannot erase the
  obligation.

  Reconcile added/removed/moved expressions by
  `(path, containing symbol, normalized expression)`. Split distinct complete
  expressions; merge only identical expressions in the same symbol. Recompute
  raw, excluded, included, union, and disposition totals. Every `continued` row
  must name the actual final reporter/warning site and passing behavioral test;
  expected rows must state the exact validation/domain or structurally
  infallible condition; propagated rows must retain their typed source.

- [ ] **Step 2: Write the inventory-check red tests**

  Use small Markdown fixtures to assert the new step rejects: a baseline
  remediation key absent from the ledger; a duplicate or unknown baseline key;
  blank task/commit/test/command/final-outcome fields; a `row:` outcome whose
  final key is absent; a continued entry without reporter or primary-result
  proof; and any surviving pending-amendment marker. Add passing fixtures for a
  matching `row:` outcome and a justified `removed:` outcome. Run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml error_swallowing_inventory
  ```

  Expected before implementation: FAIL because the step and ledger do not exist.

- [ ] **Step 3: Implement and run the mechanical reconciliation**

  Parse the classified table and delivered-remediation ledger with a narrow
  line/table parser. Require exactly the frozen 88 baseline keys once each,
  nonempty delivery fields, valid final-row references or explicit removal
  reasons, and reporter/primary fields for continuations. Also reject incomplete
  fields on any final continued inventory row. Register the step in
  `cargo xtask check`; do not inspect Rust source tokens.

  Populate the ledger from Tasks 2-6 and 11 only after their focused tests and
  commits exist, then run:

  ```bash
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml error_swallowing_inventory
  devtool run -- cargo xtask check --no-test
  ```

  Expected: PASS only when every one of the 88 remediation keys has delivered
  proof and the final inventory has no incomplete continuation.

- [ ] **Step 4: Run focused contract suites**

  ```bash
  devtool run -- cargo nextest run -p common password
  devtool run -- cargo nextest run -p common atompub
  devtool run -- cargo nextest run -p storage email_verification
  devtool run -- cargo nextest run -p storage confirm_password_reset
  devtool run -- cargo nextest run -p storage create_user_with_invite
  devtool run -- cargo nextest run -p storage continuation_reporting
  devtool run -- cargo nextest run -p host error
  devtool run -- cargo nextest run -p host capture
  devtool run -- cargo nextest run -p client telemetry
  devtool run -- cargo nextest run -p web media
  devtool run -- cargo nextest run -p web continuation_reporting
  devtool run -- cargo nextest run -p web default_post_format
  devtool run -- cargo nextest run -p web audience_picker
  devtool run -- cargo nextest run -p jaunder client_telemetry
  devtool run -- cargo nextest run -p jaunder mailer
  devtool run -- cargo nextest run -p jaunder media
  devtool run -- cargo nextest run -p jaunder projector
  devtool run -- cargo nextest run -p jaunder feed
  devtool run -- cargo nextest run -p jaunder backup
  devtool run -- cargo nextest run -p jaunder runtime_file
  devtool run -- cargo nextest run -p jaunder commands
  devtool run -- cargo nextest run -p jaunder build_script_staging
  devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml
  devtool run -- cargo nextest run --manifest-path tools/devtool/Cargo.toml
  ```

- [ ] **Step 5: Run focused browser proofs**

  ```bash
  devtool run -- cargo xtask e2e-local client-telemetry.spec.ts
  devtool run -- cargo xtask e2e-local profile.spec.ts
  devtool run -- cargo xtask e2e-local posts.spec.ts
  ```

  Expected: client warning/request-start proof, real intake diagnostic proof,
  and both #898/#899 visible failure flows pass.

- [ ] **Step 6: Run the complete local gate**

  ```bash
  devtool run -- cargo xtask check
  devtool run -- cargo xtask validate
  ```

  Expected: the inventory checker and all static checks pass; dual-backend
  coverage/tests and all four SQLite/PostgreSQL × Chromium/Firefox e2e
  combinations pass.

- [ ] **Step 7: Review final evidence and commit**

  ```bash
  git add xtask/src/steps/error_swallowing_inventory_check.rs xtask/src/lib.rs docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-inventory.md docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-audit.md docs/superpowers/plans/2026-08-13-issue-58-error-swallowing-audit.md
  git commit -m "docs: enforce reconciled error audit evidence"
  ```

- [ ] **Step 8: Deliver tracker evidence**

  In the issue #58 PR body, map AC1-AC23 to commits/tests and map every
  #898/#899 acceptance item to host and browser evidence. Close #898/#899 only
  through that PR after the mapping is complete; do not leave duplicate
  implementation work.

# Spec — Issue #28: bound coverage-build disk via per-test Postgres DB teardown

- **Issue:** jaunder-org/jaunder#28 — *ci: Validate fails from disk exhaustion in the coverage build, misattributed as a test/coverage failure*
- **Milestone:** #1 — Verify-gate hardening
- **Date:** 2026-06-25
- **Status:** approved (design)

## Problem

`Validate` on `main` fails intermittently at the `cargo xtask validate` →
`jaunder-coverage` Nix derivation. The surface error is opaque (`builder failed
with exit code 1`) and historically read as a coverage/test regression. The real
cause is **disk exhaustion**: PostgreSQL fails to extend a file with SQLSTATE
`53100` (`No space left on device`) during the instrumented test run.

CI is intermittent — alternating pass/fail on `main` — the signature of disk
sitting right at the runner's free-space threshold and tipping over when the
baseline is slightly lower.

## Root cause

The instrumented build's *compile/link* disk pressure was already addressed
(prior work set `CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0` to
stop rust-lld from exhausting the build FS). The remaining consumer is **runtime
data growth**, consistent with the failure occurring in PostgreSQL `mdextend`
(not at link time):

- `server/tests/helpers/mod.rs::template_postgres_url()` mints a fresh per-test
  database with `CREATE DATABASE <unique> TEMPLATE <template>` on every call.
- **Nothing ever drops the clone.** There is no `DROP DATABASE`, no `Drop`
  guard, and no teardown anywhere in `server/tests/helpers/`.
- The ephemeral cluster's `PGDATA` is a `mktemp -d` (`scripts/with-ephemeral-postgres`)
  on the sandbox filesystem — the same disk as the build. Every Postgres-backed
  test therefore leaves a full template-sized database (catalogs + migrated
  schema, on the order of ~8–10 MB each) behind for the entire run.
- This consumer **scales with the number of Postgres-backed test cases**, so as
  the suite grows it crosses the disk threshold.

## Scope

This issue has two acceptance criteria. They are in very different states:

### Acceptance #2 — already satisfied (no new code)

> *A regression guard or clear signal so a future ENOSPC is reported as an
> infrastructure failure, not a test/coverage failure.*

This is **already implemented and proven in production**:

- `tools/devtool/src/coverage/emit.rs::classify_nextest_output` detects the
  disk-full markers (`"53100"`, `No space left on device`) and classifies the
  run as `StatusCategory::Infra` with an `infra_detail`, written to
  `status.json`.
- The Nix `coverage-gate` derivation (`flake.nix`) prints `infra_detail` and
  fails with `category=infra`.
- xtask's `sentinel_detail` (`xtask/src/steps/nix.rs`) renders it as
  `infrastructure failure (not a coverage regression): <detail>`.
- Tests exist at every layer (`emit.rs`, `tools/coverage/src/status.rs`,
  `xtask/src/steps/nix.rs`).
- Today's CI failure already surfaces `category=infra` /
  `infra_detail: "No space left on device"` and
  `[FAIL] coverage — infrastructure failure (not a coverage regression)`.

**This spec adds no code for #2.** It only verifies the existing classification
tests still pass, so the criterion is not regressed.

### Acceptance #1 — the work of this issue

> *Root cause of the disk exhaustion identified and fixed so `Validate` passes
> on `main`.*

Bound the per-test Postgres disk growth by dropping each cloned database when its
test finishes.

## Design

### Mechanism: RAII teardown via a `base`-typed wrapper

The natural owner of a per-test DB's lifetime is the `TestEnv` returned by
`Backend::setup`. But `TestEnv` cannot carry the teardown the obvious ways:

- A `Drop` impl on `TestEnv` is impossible — tests destructure it
  (`let TestEnv { state, base } = ...`) in **240 sites**, and you cannot move
  fields out of a type that implements `Drop`.
- Adding a new guard *field* to `TestEnv` would force edits to all 240
  destructure sites (this is precisely why the original author used the
  `PG_URL_FILE` side-channel instead of a struct field).

The clean seam is to change the **type** of the existing `base` field — not add a
field:

- Introduce `TestBase` in `server/tests/helpers/`: a thin wrapper owning the
  existing `TempDir` plus an `Option<String>` holding the per-test database name
  (`None` on SQLite).
- `impl Deref<Target = TempDir> for TestBase` so `base.path()` and `&base`
  (the only ways tests use it — confirmed: 9 `&base` sites, all others
  `base.path()`; no by-value `TempDir` consumers) keep compiling unchanged.
- Change `TestEnv.base`'s type from `TempDir` to `TestBase`. **Field name
  unchanged**, so all 240 `let TestEnv { state, base }` destructures compile
  untouched.
- `Backend::setup`'s Postgres arm records the per-test DB name into the
  `TestBase`; the SQLite arm records `None`.

### Drop behavior

`impl Drop for TestBase`:

- If a DB name is present, drop it best-effort with
  `DROP DATABASE <name> WITH (FORCE)`.
  - PG16 supports `WITH (FORCE)`, which terminates any lingering connections
    (e.g. a raw-SQL pool opened via `recorded_postgres_url`). This makes the fix
    robust to field/local drop ordering relative to `state` — we do not need to
    guarantee the pool is closed first.
  - The admin connection uses the bootstrap URL `Backend::setup` already uses
    (`postgres_bootstrap_url()`), since `DROP DATABASE` must run from a different
    database than the one being dropped.
- Errors are **swallowed** (logged, not panicked): panicking in `Drop` during an
  unwind aborts the process. A failed drop at end-of-run is harmless (the whole
  cluster is discarded).

**Async-in-Drop:** `Drop` is synchronous and the drop must talk to Postgres. The
implementation spawns a short-lived `std::thread` with its own current-thread
Tokio runtime to run the sqlx drop, and joins it. This reuses sqlx (no new
dependency) and is safe inside nextest's Tokio context (a fresh thread has no
ambient runtime). The planning step will confirm this is preferable to pulling in
the synchronous `postgres` crate.

### Effect

Peak concurrent per-test databases ≈ nextest concurrency, instead of the total
number of Postgres-backed test cases. Disk no longer scales with the suite.

## Testing / verification

1. **Teardown regression test** (the core guard): create a per-test database via
   the helper, drop the `TestBase`, and assert the database no longer exists in
   the cluster.
2. **Disk-bound evidence:** measure peak `PGDATA` size while running the Postgres
   test suite (via `scripts/with-ephemeral-postgres`) before vs. after the fix —
   recorded in the plan as proof, since local disk is too ample to reproduce
   ENOSPC on its own.
3. **Acceptance #2 non-regression:** the existing classification tests
   (`emit.rs`, `status.rs`, `nix.rs`) still pass.
4. **Gate:** `cargo xtask validate` green locally; the conclusive proof is
   `Validate` going green on CI `main`.

## Risks / edge cases

- **Raw-SQL pools outliving `base`:** handled by `WITH (FORCE)`.
- **Dropping from the wrong database:** avoided by using the bootstrap/admin URL.
- **Drop during panic/unwind:** errors swallowed; no panic-in-Drop.
- **SQLite tests:** `TestBase` carries `None`; `Drop` is a no-op beyond the
  existing `TempDir` cleanup.

## Out of scope

- The failure-attribution / category work (#2) — already done.
- Any change to the build-artifact (compile/link) disk footprint — already
  addressed by the existing `CARGO_PROFILE_*_DEBUG=0` settings.
- Broader test-isolation redesign (shared DB + transaction rollback / schema
  per test) — rejected in brainstorming as a large, risky refactor.

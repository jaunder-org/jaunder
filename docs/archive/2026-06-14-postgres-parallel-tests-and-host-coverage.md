# Parallel PostgreSQL Tests, Host Coverage & VM Consolidation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make PostgreSQL integration tests run in parallel (per-test databases) like the SQLite tests, add a no-VM host-PostgreSQL coverage pass that finally measures `storage/src/postgres/*`, collapse the 23 per-binary NixOS VM checks into one, and retier `scripts/verify` into a fast no-VM local gate plus a VM-backed `--full`/CI gate.

**Success metric:** A committed before/after table (`docs/superpowers/plans/2026-06-14-perf-results.md`) showing wall-clock reductions for: the full local gate, the PostgreSQL integration run (serial→parallel), and the Nix VM phase (23 VMs→1). Phase 0 captures the baseline *before any code changes*; every later phase re-measures against it. No phase is "done" until its number is recorded.

**Architecture:** The SQLite tests are parallel because each gets its own `TempDir`/`test.db`; the PostgreSQL tests serialize (`--test-threads=1`) only because `reset_postgres_schema()` mutates one shared database. We replace that with a per-test database cloned from a once-migrated template (race-safe across nextest's process-per-test model via a PostgreSQL advisory lock). A `scripts/with-ephemeral-postgres` wrapper spins up a throwaway PG 16 cluster on the host so the same instrumented test binaries that produce SQLite coverage also run against PostgreSQL, merging both passes into one `cargo llvm-cov report`. CI keeps a single hermetic NixOS VM that runs every integration binary, narrowed to validating the deployment module.

**Tech Stack:** Rust, sqlx (PostgreSQL + SQLite), cargo-nextest, cargo-llvm-cov, cargo-crap, Nix flakes / `pkgs.testers.nixosTest`, bash.

**Sequencing:** Four phases, each independently shippable and committable. Execute in order — Phase 2 depends on the per-test-DB helpers and the ephemeral-PG script from Phase 1; Phase 3 depends on Phase 1's parallel binaries; Phase 4 wires the new gate together.

**Prerequisite for every phase:** Read `CONTRIBUTING.md` (backend parity, coverage policy, verify ladder) before starting. Work on the current `epic` branch or a feature branch off it — never `main`. Run `scripts/verify --fast` before each commit; run the relevant deeper gate where a task says so.

---

## File Structure

| File | Responsibility | Phase |
|------|----------------|-------|
| `docs/superpowers/plans/2026-06-14-perf-results.md` (create) | Performance baseline + per-phase before/after comparison table | 0, all |
| `scripts/with-ephemeral-postgres` (create) | Spin up a throwaway PG 16 cluster, export `JAUNDER_PG_TEST_URL`/`JAUNDER_PG_BOOTSTRAP_TEST_URL`, run a command, tear down | 1 |
| `server/tests/helpers/mod.rs` (modify) | Add template + per-test-DB helpers; reroute `test_state*` to per-test DBs; remove `reset_postgres_schema` | 1 |
| `server/tests/storage.rs` (modify) | Convert the migration-behavior tests off the shared DB onto fresh per-test DBs | 1 |
| `.config/nextest.toml` (modify) | Optional serial test-group fallback if any cluster-global collision appears | 1 |
| `scripts/check-coverage` (modify) | Two-pass coverage: SQLite pass + host-PostgreSQL pass, merged into one report | 2 |
| `.coverage-manifest.json`, `.crap-manifest.json` (modify) | Regenerated baseline now including PostgreSQL code | 2 |
| `CONTRIBUTING.md` (modify) | Update coverage carve-out notes and the Nix VM checks section | 2, 3 |
| `flake.nix` (modify) | Replace per-binary `postgresTestBinaryCheck` map with one consolidated VM; trim VM memory | 3 |
| `.github/workflows/e2e.yml` (modify) | Point the postgres-integration job at the consolidated check | 3 |
| `scripts/verify` (modify) | Retier: `--fast`, default (no VM), `--full` (VM) | 4 |

---

## Phase 0 — Capture the performance baseline

**Outcome:** A committed results file with current timings, captured *before any code change*, so every later phase has something to measure against. **Do this first — once Phase 1 lands, the old serial/23-VM numbers are gone.**

**Measurement protocol (read before timing):**
- **Caches matter more than anything.** Compilation and Nix builds are cached, so a naive re-run measures a cache hit, not real work. Warm the cargo target *once* before timing host steps (so we measure test execution, not first compile). For Nix VM steps, the developer-experienced cost is the *cold* build — force it (commands below) rather than timing a cache hit.
- **Repeat host steps 3×, report the median**, plus min/max. Nix cold builds are timed once (forcing them is expensive); note that.
- **Record the environment**: `nproc`, total RAM, and whether the run saturated CPU/OOM'd — that context is the reason the work exists.

### Task 0: Record baseline timings

**Files:**
- Create: `docs/superpowers/plans/2026-06-14-perf-results.md`

- [ ] **Step 1: Create the results file with the environment and an empty table**

```markdown
# Performance results — PostgreSQL parallel tests / host coverage / VM consolidation

## Environment
- Host: <`nproc` cores, `free -h` total RAM>
- Caches: cargo target warm (built once before timing); Nix store as noted per row
- Date captured: 2026-06-14

## Results (wall clock; host rows = median of 3, min–max in parens)

| Metric | Baseline | After Phase 1 | After Phase 3 | Final |
|--------|----------|---------------|---------------|-------|
| `scripts/verify --fast` | | — | — | |
| `scripts/check-coverage` (host, SQLite only at baseline) | | | — | |
| PostgreSQL integration run — serial (`--test-threads=1`) | | | — | — |
| PostgreSQL integration run — parallel (default threads) | — | | — | |
| One postgres VM check (cold, e.g. `postgres-commands`) | | — | n/a | n/a |
| Postgres integration VMs total (`postgres-integration-checks`, cold) | | — | | |
| `nix build .#nix-only-checks` (cold) | | — | | |
| Full `scripts/verify` (default tier) | | | | |
| Full `scripts/verify --full` (with VM) | n/a | | | |
| Peak RSS / OOM notes | | | | |
```

- [ ] **Step 2: Record the environment**

Run: `printf 'cores=%s\n' "$(nproc)"; free -h | awk '/Mem:/{print "ram="$2}'`
Paste the values into the Environment section.

- [ ] **Step 3: Warm the build caches (so host timings measure execution, not first compile)**

Run: `nix develop --command cargo build --workspace --tests`
Expected: completes; subsequent host timings now reflect warm-cache runs.

- [ ] **Step 4: Time the host steps (3 runs each, record median + min–max)**

Run (repeat each 3×):
```bash
for i in 1 2 3; do { /usr/bin/time -v scripts/verify --fast ; } 2>>perf-fast.log ; done
for i in 1 2 3; do { /usr/bin/time -v nix develop --command scripts/check-coverage ; } 2>>perf-cov.log ; done
```
Read the wall-clock from `Elapsed (wall clock) time` and peak memory from `Maximum resident set size` in each log. (`/usr/bin/time` is GNU time; if absent, add `pkgs.time` to the devShell `buildInputs`, or fall back to bash `{ time CMD; }`.) Record medians in the table.

- [ ] **Step 5: Time the serial PostgreSQL run (the thing Phase 1 replaces), apples-to-apples on the host**

Run:
```bash
nix develop --command scripts/with-ephemeral-postgres \
    cargo nextest run -p jaunder --include-ignored --test-threads=1
```
Wait — `scripts/with-ephemeral-postgres` is created in Phase 1 Task 1. **Pull that one task forward**: implement Task 1 (the wrapper script) before this step, commit it, then run the line above. Record the wall time as the serial baseline. (The parallel number is filled in after Phase 1.)

- [ ] **Step 6: Time the Nix VM phase — cold (force a real rebuild, not a cache hit)**

A single postgres VM (per-binary cost):
```bash
nix build --accept-flake-config --rebuild .#checks.x86_64-linux.postgres-commands 2> perf-pg1.log
```
The whole VM phase, cold — build once to populate, delete the outputs, then time the rebuild:
```bash
nix build --accept-flake-config .#nix-only-checks --no-link
nix store delete $(nix path-info --recursive .#nix-only-checks 2>/dev/null) 2>/dev/null || true
{ /usr/bin/time -v nix build --accept-flake-config .#nix-only-checks ; } 2> perf-nixall.log
```
Record the per-VM and aggregate wall times, the postgres VM **count** (23) for context, and observed Nix parallelism / any OOM. (`nix store delete` skips paths still referenced by other GC roots; `|| true` keeps going. This is the honest "cost after a source change" number a developer pays.)

- [ ] **Step 7: Time the full baseline gate**

Run: `{ /usr/bin/time -v scripts/verify ; } 2> perf-verify-full.log`
Record total wall + peak RSS. Note whether it saturated CPU or approached OOM.

- [ ] **Step 8: Fill the Baseline column, then remove the scratch logs**

Run: `rm -f perf-*.log`
Then commit:

```bash
git add docs/superpowers/plans/2026-06-14-perf-results.md scripts/with-ephemeral-postgres
git commit -m "perf: capture testing-time baseline before optimization"
```

---

## Phase 1 — Parallel per-test PostgreSQL databases

> **Note:** Task 1 (the `scripts/with-ephemeral-postgres` wrapper) was pulled forward into Phase 0 Step 5. If it is already committed, skip Task 1 below and proceed to Task 2.


**Outcome:** `cargo nextest run -p jaunder` against a live PostgreSQL passes with default (parallel) threads — no `--test-threads=1` — and produces the same results as the SQLite run.

### Task 1: Ephemeral host-PostgreSQL wrapper script

**Files:**
- Create: `scripts/with-ephemeral-postgres`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Run a command with a throwaway PostgreSQL 16 cluster.
#
# Spins up an initdb cluster in a temp dir on a private port, creates the
# `jaunder` application role/database and leaves `postgres` as the superuser,
# exports JAUNDER_PG_TEST_URL + JAUNDER_PG_BOOTSTRAP_TEST_URL, runs the given
# command, then stops the cluster and deletes the temp dir on exit.
#
# Durability is intentionally disabled (fsync/synchronous_commit off): the
# cluster is discarded after the run, so we trade crash-safety for speed.
#
# Usage:
#   scripts/with-ephemeral-postgres cargo nextest run -p jaunder
set -euo pipefail

PGDATA="$(mktemp -d -t jaunder-pg.XXXXXX)"
PGPORT="${JAUNDER_PG_TEST_PORT:-54329}"
PGHOST=127.0.0.1

cleanup() {
    pg_ctl -D "$PGDATA" -m immediate stop >/dev/null 2>&1 || true
    rm -rf "$PGDATA"
}
trap cleanup EXIT INT TERM

initdb -D "$PGDATA" -U postgres --no-sync >/dev/null

pg_ctl -D "$PGDATA" -w start >/dev/null -o "$(printf '%s' \
    "-c listen_addresses=$PGHOST " \
    "-p $PGPORT " \
    "-c max_connections=200 " \
    "-c fsync=off " \
    "-c full_page_writes=off " \
    "-c synchronous_commit=off")"

psql -h "$PGHOST" -p "$PGPORT" -U postgres -d postgres -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
CREATE ROLE jaunder LOGIN CREATEDB;
CREATE DATABASE jaunder OWNER jaunder;
SQL

export JAUNDER_PG_TEST_URL="postgres://jaunder@${PGHOST}:${PGPORT}/jaunder"
export JAUNDER_PG_BOOTSTRAP_TEST_URL="postgres://postgres@${PGHOST}:${PGPORT}/postgres"

status=0
"$@" || status=$?
exit "$status"
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x scripts/with-ephemeral-postgres`

- [ ] **Step 3: Smoke-test the cluster lifecycle (no app code yet)**

Run: `nix develop --command scripts/with-ephemeral-postgres bash -c 'psql "$JAUNDER_PG_TEST_URL" -tAc "SELECT 1"'`
Expected: prints `1`, then the temp dir is removed (no leftover `jaunder-pg.*` in `$TMPDIR`).

- [ ] **Step 4: Confirm cleanup on failure**

Run: `nix develop --command scripts/with-ephemeral-postgres bash -c 'exit 7'; echo "exit=$?"`
Expected: `exit=7`, and `ls "${TMPDIR:-/tmp}"/jaunder-pg.* 2>/dev/null` shows nothing.

- [ ] **Step 5: Commit**

```bash
git add scripts/with-ephemeral-postgres
git commit -m "test(pg): add ephemeral host-PostgreSQL wrapper script"
```

### Task 2: Template + per-test-database helpers

**Files:**
- Modify: `server/tests/helpers/mod.rs` (add helpers near the existing `unique_postgres_url`, lines 144-205)

The existing `unique_postgres_db_name()`, `postgres_url_with_db_name()`, `quote_postgres_identifier()`, and `unique_postgres_url()` stay. We add a once-migrated template and a clone helper. nextest runs each test in its own **process**, so the template must be created race-safely across processes — we use a PostgreSQL advisory lock, not a process-local `OnceCell`.

- [ ] **Step 1: Add the template constants and `ensure_template_db()`**

Insert after `unique_postgres_url()` (after line 205):

```rust
/// Name of the once-migrated template database that per-test databases are
/// cloned from. Cloning via `CREATE DATABASE ... TEMPLATE` block-copies an
/// already-migrated schema, so individual tests pay a fast copy instead of
/// re-running every migration.
const TEMPLATE_DB: &str = "jaunder_test_template";

/// Advisory-lock key serializing template creation across nextest's
/// process-per-test workers. The first worker to run migrates the template;
/// the rest see it already exists and skip straight to cloning.
const TEMPLATE_LOCK_KEY: i64 = 78_316_621;

/// Ensures `TEMPLATE_DB` exists and is fully migrated. Safe to call
/// concurrently from many processes: creation is guarded by a session-level
/// advisory lock on the bootstrap connection.
async fn ensure_template_db() {
    use sqlx::{Connection, PgConnection, PgPool};

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
    let mut admin = PgConnection::connect_with(&bootstrap).await.unwrap();

    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(TEMPLATE_LOCK_KEY)
        .execute(&mut admin)
        .await
        .unwrap();

    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(TEMPLATE_DB)
            .fetch_one(&mut admin)
            .await
            .unwrap();

    if !exists {
        let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
            panic!("expected postgres options");
        };
        let owner = options.get_username();
        sqlx::query(&format!(
            "CREATE DATABASE {} OWNER {}",
            quote_postgres_identifier(TEMPLATE_DB),
            quote_postgres_identifier(owner),
        ))
        .execute(&mut admin)
        .await
        .unwrap();

        // Migrate the template through its own pool, then close it: a database
        // may only serve as a CREATE DATABASE template when nobody is connected.
        let pool = PgPool::connect(&postgres_url_with_db_name(TEMPLATE_DB))
            .await
            .unwrap();
        sqlx::migrate!("../storage/migrations/postgres")
            .run(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(TEMPLATE_LOCK_KEY)
        .execute(&mut admin)
        .await
        .unwrap();
}

/// Creates a fresh, already-migrated per-test database cloned from the template
/// and returns its connection options. The database is owned by the same role
/// as the configured test URL so the application user can access every object.
pub async fn template_postgres_url() -> DbConnectOptions {
    use sqlx::Connection;

    ensure_template_db().await;

    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options");
    };
    let owner = options.get_username();
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
    let mut admin = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {} OWNER {} TEMPLATE {}",
        quote_postgres_identifier(&db_name),
        quote_postgres_identifier(owner),
        quote_postgres_identifier(TEMPLATE_DB),
    ))
    .execute(&mut admin)
    .await
    .unwrap();

    postgres_url_with_db_name(&db_name).parse().unwrap()
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo nextest list -p jaunder --tests >/dev/null`
Expected: lists targets with no compile error. (`template_postgres_url` is unused for now; `#![allow(dead_code)]` at the top of the file covers that.)

- [ ] **Step 3: Commit**

```bash
git add server/tests/helpers/mod.rs
git commit -m "test(pg): add migrated-template clone helper for per-test databases"
```

### Task 3: Reroute `test_state` and `test_state_with_mailer` onto per-test databases

**Files:**
- Modify: `server/tests/helpers/mod.rs:207-214` (`test_state`)
- Modify: `server/tests/helpers/mod.rs:216-273` (`test_state_with_mailer`)

- [ ] **Step 1: Import `open_existing_database`**

In the `use storage::{...}` block (lines 18-27), add `open_existing_database` next to `open_database`:

```rust
use storage::{
    open_database, open_existing_database, AppState, DbConnectOptions, PostgresAtomicOps,
    PostgresEmailVerificationStorage, PostgresFeedCacheStorage, PostgresFeedEventStorage,
    PostgresInviteStorage, PostgresMediaStorage, PostgresPasswordResetStorage, PostgresPostStorage,
    PostgresSessionStorage, PostgresSiteConfigStorage, PostgresUserConfigStorage,
    PostgresUserStorage, SqliteAtomicOps, SqliteEmailVerificationStorage, SqliteFeedCacheStorage,
    SqliteFeedEventStorage, SqliteInviteStorage, SqliteMediaStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserConfigStorage,
    SqliteUserStorage,
};
```

- [ ] **Step 2: Rewrite the `test_state` PostgreSQL branch**

Replace lines 207-214:

```rust
pub async fn test_state(base: &TempDir) -> Arc<AppState> {
    if postgres_testing_enabled() {
        let url = template_postgres_url().await;
        open_existing_database(&url).await.unwrap()
    } else {
        open_database(&sqlite_url(base)).await.unwrap()
    }
}
```

- [ ] **Step 3: Rewrite the `test_state_with_mailer` PostgreSQL branch**

Replace the `if postgres_testing_enabled()` block (lines 218-242) so it clones a per-test DB instead of resetting the shared schema. The template is already migrated, so drop the inline `sqlx::migrate!`:

```rust
    let state = if postgres_testing_enabled() {
        let DbConnectOptions::Postgres { options, .. } = template_postgres_url().await else {
            panic!("expected postgres options");
        };
        let pool = sqlx::PgPool::connect_with(options).await.unwrap();
        Arc::new(AppState {
            site_config: Arc::new(PostgresSiteConfigStorage::new(pool.clone())),
            users: Arc::new(PostgresUserStorage::new(pool.clone())),
            sessions: Arc::new(PostgresSessionStorage::new(pool.clone())),
            invites: Arc::new(PostgresInviteStorage::new(pool.clone())),
            atomic: Arc::new(PostgresAtomicOps::new(pool.clone())),
            email_verifications: Arc::new(PostgresEmailVerificationStorage::new(pool.clone())),
            password_resets: Arc::new(PostgresPasswordResetStorage::new(pool.clone())),
            posts: Arc::new(PostgresPostStorage::new(pool.clone())),
            media: Arc::new(PostgresMediaStorage::new(pool.clone())),
            user_config: Arc::new(PostgresUserConfigStorage::new(pool.clone())),
            feed_cache: Arc::new(PostgresFeedCacheStorage::new(pool.clone())),
            feed_events: Arc::new(PostgresFeedEventStorage::new(pool)),
            websub: Arc::new(common::websub::NoopWebSubClient),
        })
    } else {
```

(The SQLite `else` branch and the rest of the function are unchanged.)

- [ ] **Step 4: Compile**

Run: `cargo nextest list -p jaunder --tests >/dev/null`
Expected: no compile error.

- [ ] **Step 5: Run the SQLite suite (must stay green; no PG yet)**

Run: `cargo nextest run -p jaunder`
Expected: PASS — proves the refactor didn't disturb the default backend.

- [ ] **Step 6: Commit**

```bash
git add server/tests/helpers/mod.rs
git commit -m "test(pg): clone a per-test database in test_state helpers"
```

### Task 4: Convert `storage.rs` migration-behavior tests off the shared database

**Files:**
- Modify: `server/tests/storage.rs` — the local `postgres_state()` helper (around line 49) and the migration-behavior tests at lines 364, 371, 379, 413-418.

These tests are special: several deliberately exercise *running migrations on an empty database*, so they must each get a fresh **un-migrated** database (use the existing `unique_postgres_url()`), not a template clone. The general-purpose `postgres_state()` helper wants a migrated DB, so it uses the template clone.

- [ ] **Step 1: Update the imports in `storage.rs`**

Change line 29 from:

```rust
use helpers::{postgres_url, reset_postgres_schema, sqlite_url};
```

to:

```rust
use helpers::{sqlite_url, template_postgres_url, unique_postgres_url};
```

(`postgres_url` and `reset_postgres_schema` are dropped here; if `postgres_url` is still referenced elsewhere in the file, keep it in the import list — confirm with the compile in Step 5.)

- [ ] **Step 2: Rewrite the local `postgres_state()` helper**

Replace its body (around lines 49-51) so it returns a migrated per-test DB:

```rust
async fn postgres_state() -> std::sync::Arc<storage::AppState> {
    let url = template_postgres_url().await;
    storage::open_existing_database(&url).await.unwrap()
}
```

- [ ] **Step 3: Rewrite the three migration-behavior tests to use a fresh empty DB**

For `open_database_succeeds_on_postgres_test_vm` (line 364), `open_database_runs_postgres_migrations_on_existing_empty_db` (line 371), and `open_existing_database_runs_postgres_migrations_on_unmigrated_db` (line 379): replace the leading `reset_postgres_schema().await;` + use of `postgres_url()` with a per-test empty database. Pattern (apply to each, adapting the open call already present):

```rust
    // was: reset_postgres_schema().await;  let opts = postgres_url();
    let opts = unique_postgres_url().await;
    // ...the rest of the test body uses `opts` exactly as it used `postgres_url()`.
```

- [ ] **Step 4: Rewrite `postgres_authenticate_with_corrupted_hash_returns_internal_error` (lines 413-418)**

It currently does `reset_postgres_schema().await;` then connects and runs `sqlx::migrate!`. Replace with a template clone (already migrated) and drop the manual migrate:

```rust
    let DbConnectOptions::Postgres { options, .. } = template_postgres_url().await else {
        panic!("expected postgres options");
    };
    let pool = sqlx::PgPool::connect_with(options).await.unwrap();
    // (no sqlx::migrate! — the template is already migrated)
```

Add `use storage::DbConnectOptions;` to the test module imports if not already present (confirm in Step 5).

- [ ] **Step 5: Confirm no remaining `reset_postgres_schema` callers anywhere**

Run: `grep -rn "reset_postgres_schema" server/tests`
Expected: **no matches.** If any remain, convert them with the same patterns (template clone for "wants a migrated DB", `unique_postgres_url` for "wants an empty DB") before continuing.

- [ ] **Step 6: Delete `reset_postgres_schema` from the helper**

Remove `reset_postgres_schema()` (helpers/mod.rs:128-142). Then:

Run: `cargo nextest list -p jaunder --tests >/dev/null`
Expected: compiles with no "unused"/"not found" errors.

- [ ] **Step 7: Commit**

```bash
git add server/tests/storage.rs server/tests/helpers/mod.rs
git commit -m "test(pg): give storage migration tests isolated per-test databases"
```

### Task 5: Run the PostgreSQL suite in parallel and prove isolation

**Files:** none (verification + possible `.config/nextest.toml` fallback)

- [ ] **Step 1: Run the full PostgreSQL integration suite in parallel**

Run:
```bash
nix develop --command scripts/with-ephemeral-postgres \
    cargo nextest run -p jaunder --include-ignored
```
Expected: PASS with default parallelism (no `--test-threads=1`). Watch for: "database is being accessed by other users", duplicate-key errors on cluster-global objects, or role-collision failures.

- [ ] **Step 2: Run it three times to catch order/parallelism flakes**

Run:
```bash
for i in 1 2 3; do \
  nix develop --command scripts/with-ephemeral-postgres \
    cargo nextest run -p jaunder --include-ignored || { echo "FAILED on run $i"; break; }; \
done
```
Expected: three clean passes.

- [ ] **Step 3: Record the parallel timing and compare to the serial baseline**

Run (3×, median):
```bash
nix develop --command scripts/with-ephemeral-postgres \
    cargo nextest run -p jaunder --include-ignored
```
Enter the median into the "PostgreSQL integration run — parallel" / "After Phase 1" cell of `2026-06-14-perf-results.md`, alongside the serial baseline from Phase 0 Step 5. Also re-time `scripts/check-coverage` (still SQLite-only here) and `scripts/verify --fast` into the "After Phase 1" column. This is the isolated parallelism win; record the speedup ratio.

- [ ] **Step 4 (only if Step 1/2 shows a cluster-global collision): add a serial test-group fallback**

If — and only if — the role/provisioning tests in `commands.rs`/`storage.rs` collide (they already use unique `jaunder_role_{suffix}`/`jaunder_db_{suffix}` names, so this is unlikely), serialize just those in `.config/nextest.toml`:

```toml
[test-groups]
pg-cluster-global = { max-threads = 1 }

[[profile.default.overrides]]
# Tests that create cluster-global roles/databases must not run concurrently.
filter = 'package(jaunder) and test(create_pg_db)'
test-group = 'pg-cluster-global'
```

Run: `nix develop --command scripts/with-ephemeral-postgres cargo nextest run -p jaunder --include-ignored`
Expected: PASS.

- [ ] **Step 5: Commit (the perf-results update, plus the nextest config if Step 4 was needed)**

```bash
git add -A
git commit -m "test(pg): verify parallel PostgreSQL run is isolation-clean; record timing"
```

---

## Phase 2 — No-VM host coverage including PostgreSQL code

**Outcome:** `scripts/check-coverage` runs the suite twice (SQLite, then host PostgreSQL) under one llvm-cov session and reports merged coverage, so `storage/src/postgres/*.rs` and `storage/src/backup/postgres.rs` get real numbers.

### Task 6: Two-pass coverage in `check-coverage`

**Files:**
- Modify: `scripts/check-coverage:62-77` (the report-generation block)

cargo-llvm-cov supports accumulating multiple runs with `--no-report` and emitting a single merged `report`. We run the default SQLite pass, then the PostgreSQL pass (scoped to the `jaunder` integration tests, `--include-ignored`) inside the ephemeral cluster, then merge.

- [ ] **Step 1: Replace the single `cargo llvm-cov nextest` invocation with the two-pass flow**

In the `else` branch (the non-`--investigate` path), replace lines 63-71:

```bash
    echo "--- coverage: running cargo llvm-cov (SQLite pass) ---"
    cargo llvm-cov nextest --no-report --show-progress none

    echo "--- coverage: running cargo llvm-cov (PostgreSQL pass) ---"
    scripts/with-ephemeral-postgres \
        cargo llvm-cov nextest --no-report --show-progress none \
        -p jaunder --include-ignored

    echo "--- coverage: building merged text + LCOV reports ---"
    cargo llvm-cov report --text > "$REPORT_FILE"
    cargo llvm-cov report --lcov --output-path "$LCOV_FILE" > /dev/null
```

The `--no-report` flag tells each pass to collect profraw without emitting a report; the final `report` merges all collected data. Line coverage is a union of hits, so PostgreSQL-only lines flip to covered while shared lines stay covered — no double-counting (the same reason this script avoids `--json` summary percentages).

- [ ] **Step 2: Dry-run the new coverage pass and confirm PostgreSQL files now show real numbers**

Run: `nix develop --command scripts/check-coverage --update`
Expected: completes; then inspect the manifest delta:

Run: `git --no-pager diff .coverage-manifest.json | grep -i "postgres" | head`
Expected: `storage/src/postgres/*.rs` entries jump well above their former 3–15%.

- [ ] **Step 3: Sanity-check the gate path (no `--update`) is green against the new baseline**

Run: `nix develop --command scripts/check-coverage`
Expected: "Coverage and CRAP OK ..."

- [ ] **Step 4: Commit (script + regenerated baseline together, per CONTRIBUTING.md:187)**

```bash
git add scripts/check-coverage .coverage-manifest.json .crap-manifest.json
git commit -m "test(coverage): add host-PostgreSQL pass; measure postgres storage code"
```

### Task 7: Update the coverage policy docs

**Files:**
- Modify: `CONTRIBUTING.md` — the "inherent host-side coverage gaps" list (around lines 189-193)

- [ ] **Step 1: Revise the PostgreSQL-only carve-out**

Replace the bullet that exempts `storage/src/postgres/*.rs` (and `storage/src/backup/postgres.rs`) from coverage. It is no longer a gap — it is measured on the host. Reword to state PostgreSQL storage is now covered by the host-PostgreSQL coverage pass (`scripts/check-coverage` via `scripts/with-ephemeral-postgres`), and remove the "3–15%" exemption. Leave the Leptos page-component gap as-is (closing that is a separate, optional follow-on).

- [ ] **Step 2: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs: PostgreSQL storage is now covered by the host coverage pass"
```

---

## Phase 3 — Consolidate the 23 per-binary VMs into one

**Outcome:** `nix build .#packages.x86_64-linux.postgres-integration-checks` boots **one** NixOS VM that runs every integration binary against PostgreSQL (now in parallel, thanks to Phase 1), instead of one VM per binary.

### Task 8: Replace `postgresTestBinaryCheck` with a single consolidated VM check

**Files:**
- Modify: `flake.nix` — `postgresTestBinaryCheck` (lines 547-604), the `checks` map over `integrationTestModules` (lines 922-939), and `postgres-integration-checks` (lines 876-881).

- [ ] **Step 1: Add a consolidated check builder**

Replace the `postgresTestBinaryCheck = { ... }` definition (lines 547-604) with a single VM that loops over all binaries. Each binary already creates its own per-test databases (Phase 1), so they run with default parallelism; the bootstrap URL is provided for the `--include-ignored` provisioning tests:

```nix
        # One NixOS VM runs every integration test binary against a live
        # PostgreSQL. Per-test databases (cloned from a migrated template by the
        # test harness) give each test its own namespace, so binaries run with
        # nextest's default parallelism instead of one-VM-per-binary.
        postgresIntegrationCheck = pkgs.testers.nixosTest {
          name = "jaunder-postgres-integration";

          nodes.machine =
            { pkgs, lib, ... }:
            {
              virtualisation.memorySize = 2048;
              virtualisation.diskSize = 4096;

              services.postgresql = {
                enable = true;
                package = pkgs.postgresql_16;
                ensureDatabases = [ "jaunder" ];
                ensureUsers = [
                  {
                    name = "jaunder";
                    ensureDBOwnership = true;
                  }
                ];
                authentication = ''
                  local all all trust
                  host all all 0.0.0.0/0 trust
                '';
                settings = {
                  listen_addresses = lib.mkForce "*";
                  max_connections = 200;
                };
              };

              environment.systemPackages = [ pkgs.postgresql_16 ];
            };

          testScript = ''
            machine.start()
            machine.wait_for_unit("postgresql.service", timeout=60)
            machine.wait_until_succeeds(
              "sudo -u postgres psql -tAc \"SELECT 1 FROM pg_roles WHERE rolname = 'jaunder'\" | grep -q 1"
            )
            machine.succeed(
              "sudo -u postgres psql -tAc \"ALTER ROLE jaunder CREATEDB\""
            )
            machine.wait_until_succeeds(
              "sudo -u postgres psql -tAc \"SELECT 1 FROM pg_database WHERE datname = 'jaunder'\" | grep -q 1"
            )
          ''
          + pkgs.lib.concatMapStrings (m: ''
            machine.succeed(
              "JAUNDER_PG_TEST_URL=postgres://jaunder@127.0.0.1/jaunder"
              + " JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@127.0.0.1/postgres"
              + " ${postgresIntegrationTests}/tests/${m} --include-ignored"
            )
          '') integrationTestModules;
        };
```

Notes for the implementer:
- `--test-threads=1` is intentionally gone. The test binary is the standard libtest harness; per-test databases make parallel runs safe.
- `ALTER ROLE jaunder CREATEDB` lets the `jaunder` app role create the per-test databases and the template (the harness connects as the bootstrap superuser `postgres` for `CREATE DATABASE`, but granting CREATEDB keeps both paths working).
- The old `testModuleOverrides` (`includeIgnored`, `extraEnv`) collapse into "always `--include-ignored`, always provide the bootstrap URL," so that table (lines 496-506) is no longer needed — delete it.

- [ ] **Step 2: Replace the per-binary `checks` map and the meta-package**

In the `checks` attrset, delete the `pkgs.lib.listToAttrs (map ... integrationTestModules)` block (lines 922-939) that created `postgres-<m>` checks, and instead add a single named check:

```nix
            postgres-integration = postgresIntegrationCheck;
```

Then update `postgres-integration-checks` (lines 876-881) to point at it:

```nix
          postgres-integration-checks = pkgs.symlinkJoin {
            name = "jaunder-postgres-integration-checks";
            paths = [ self.checks.${system}.postgres-integration ];
          };
```

- [ ] **Step 3: Trim the e2e VM memory while here (optional, same OOM concern)**

Leave `mkE2eSqliteCheck`/`mkE2ePostgresCheck` at `memorySize = 2048` (already modest). No change required unless benchmarking shows headroom to lower further.

- [ ] **Step 4: Evaluate the flake and build the consolidated check**

Run: `nix flake check --no-build 2>&1 | tail -20` (catches eval errors fast)
Expected: no evaluation errors.

Run: `nix build --accept-flake-config .#packages.x86_64-linux.postgres-integration-checks`
Expected: builds and passes — one VM, all binaries.

- [ ] **Step 5: Re-measure the VM phase cold and record against the 23-VM baseline**

Force a cold rebuild of the consolidated check and the whole VM phase, the same way Phase 0 Step 6 did:
```bash
nix build --accept-flake-config .#nix-only-checks --no-link
nix store delete $(nix path-info --recursive .#nix-only-checks 2>/dev/null) 2>/dev/null || true
{ /usr/bin/time -v nix build --accept-flake-config .#nix-only-checks ; } 2> perf-nixall-after.log
```
Enter the consolidated `postgres-integration-checks` and `nix-only-checks` cold times into the "After Phase 3" column of `2026-06-14-perf-results.md`, next to the 23-VM baseline. Record the reduction and any change in peak memory / OOM behavior (the headline resource win). Then `rm -f perf-nixall-after.log`.

- [ ] **Step 6: Commit**

```bash
git add flake.nix docs/superpowers/plans/2026-06-14-perf-results.md
git commit -m "ci(pg): run all integration binaries in one consolidated VM; record timing"
```

### Task 9: Update CI workflow and the Nix VM docs

**Files:**
- Modify: `.github/workflows/e2e.yml` (postgres-integration job, around lines 38-58)
- Modify: `CONTRIBUTING.md` — "Nix VM checks" section (around lines 196-230)

- [ ] **Step 1: Confirm the CI job still targets the right package**

The job runs `nix build ... .#packages.x86_64-linux.postgres-integration-checks`, which now resolves to the single consolidated check. No edit needed unless the job referenced individual `postgres-<m>` checks — verify with:

Run: `grep -n "postgres-" .github/workflows/e2e.yml`
Expected: only the `postgres-integration-checks` package reference. If any per-module names appear, replace them with `postgres-integration-checks`.

- [ ] **Step 2: Update the CONTRIBUTING Nix VM list**

In the "Nix VM checks" section, replace the enumerated `postgres-commands` / `postgres-storage` / `postgres-web-*` bullets with a single line describing `checks.x86_64-linux.postgres-integration` (all integration binaries against PostgreSQL in one VM). Note that local PostgreSQL coverage/iteration now runs without a VM via `scripts/with-ephemeral-postgres` (Phase 1/2).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/e2e.yml CONTRIBUTING.md
git commit -m "docs,ci: document the consolidated PostgreSQL integration VM"
```

---

## Phase 4 — Retier `scripts/verify`

**Outcome:** Default `scripts/verify` is the fast no-VM local gate (static + lint + host coverage incl. PostgreSQL); `scripts/verify --full` adds the hermetic Nix VM checks; `--fast` is unchanged. CI keeps running the VM checks.

### Task 10: Add the `--full` tier and move the VM build behind it

**Files:**
- Modify: `scripts/verify` (the FAST flag parsing at lines 23-33, the coverage step at 153, and the Nix step at 155-159)

- [ ] **Step 1: Parse a `--full` flag alongside `--fast`**

Replace the arg-parsing loop (lines 23-33):

```bash
FAST=0
FULL=0
for arg in "$@"; do
    case "$arg" in
        --fast) FAST=1 ;;
        --full) FULL=1 ;;
        *)
            echo "error: unknown argument: $arg" >&2
            echo "usage: scripts/verify [--fast|--full]" >&2
            exit 2
            ;;
    esac
done
```

- [ ] **Step 2: Run coverage through the ephemeral PostgreSQL wrapper**

Replace the Phase 3 coverage step (line 153):

```bash
# ── Phase 3: Coverage (full test suite: SQLite + host PostgreSQL) ─────

run_step "scripts/check-coverage" \
    scripts/with-ephemeral-postgres scripts/check-coverage
```

(Note: if `check-coverage` already invokes `with-ephemeral-postgres` internally for its PostgreSQL pass per Phase 2 Task 6, do **not** double-wrap — wrap only at one layer. Choose the internal wrap (Phase 2) as canonical and keep this step as `run_step "scripts/check-coverage" scripts/check-coverage`. Confirm which layer wraps before committing.)

- [ ] **Step 3: Gate the Nix VM build behind `--full`**

Replace the Phase 4 block (lines 155-160):

```bash
# ── Phase 4: Nix VM checks (hermetic e2e + PostgreSQL) — opt-in ───────

if [[ "$FULL" == "1" ]]; then
    run_step "nix build .#nix-only-checks" \
        nix build --accept-flake-config .#nix-only-checks
    echo "--- verify: all checks passed (incl. Nix VM) ---"
else
    echo "--- verify: local checks passed (skipped Nix VM; run 'scripts/verify --full' or rely on CI for the hermetic gate) ---"
fi
```

- [ ] **Step 4: Update the script's usage header**

Edit the comment block (lines 4-11) to document the three tiers: `--fast` (static + lint), default (adds host coverage incl. PostgreSQL, no VM), `--full` (adds the Nix VM checks). State that CI always runs `--full`-equivalent checks, so the VM tier is opt-in locally.

- [ ] **Step 5: Exercise all three tiers**

Run: `scripts/verify --fast`
Expected: stops after clippy, "fast checks passed".

Run: `scripts/verify`
Expected: runs through host coverage (SQLite + PostgreSQL), stops before the VM, "local checks passed".

Run: `scripts/verify --full`
Expected: also builds `nix-only-checks`, "all checks passed (incl. Nix VM)".

- [ ] **Step 6: Commit**

```bash
git add scripts/verify
git commit -m "build: retier verify into fast / default (no VM) / --full (VM) gates"
```

### Task 11: Update the verify-ladder docs

**Files:**
- Modify: `CONTRIBUTING.md` — the verify-ladder description (around lines 99-104) and `CLAUDE.md`/agent notes if they reference the two-rung ladder.

- [ ] **Step 1: Document the three-tier ladder**

Update the ladder text: `--fast` while iterating → default `scripts/verify` (no VM, includes host PostgreSQL coverage) as the normal pre-push gate → `scripts/verify --full` (or CI) for the hermetic VM certification. Make clear the default gate now exercises PostgreSQL on the host, so backend parity is checked locally without a VM, and the VM remains the source of truth for the NixOS deployment module.

- [ ] **Step 2: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs: describe the three-tier verify ladder"
```

### Task 12: Final before/after performance report

**Files:**
- Modify: `docs/superpowers/plans/2026-06-14-perf-results.md`

- [ ] **Step 1: Time the new gate tiers (warm caches; 3× median for host, cold for VM)**

Run:
```bash
{ /usr/bin/time -v scripts/verify ; } 2> perf-verify-default.log          # default tier, no VM
{ /usr/bin/time -v scripts/verify --full ; } 2> perf-verify-full.log      # adds the consolidated VM
```
For `--full`, force the VM cold first (Phase 3 Step 5 delete trick) so the number reflects a real post-change run rather than a cache hit.

- [ ] **Step 2: Fill the "Final" column and write a one-paragraph summary**

Complete every "Final" cell. Above the table, add 3–4 sentences stating the headline outcomes: full-gate wall-clock before→after, the serial→parallel PostgreSQL speedup, the 23-VM→1-VM reduction, the peak-memory/OOM change, and that PostgreSQL storage coverage went from waived to measured. Flag any metric that did **not** improve as expected, with a hypothesis.

- [ ] **Step 3: Commit**

```bash
rm -f perf-*.log
git add docs/superpowers/plans/2026-06-14-perf-results.md
git commit -m "perf: record final before/after testing-time results"
```

---

## Optional follow-on (not in scope, noted for later)

**Leptos page-component coverage via instrumented host e2e.** Phase 2 closes the PostgreSQL coverage gap; the other documented gap (Leptos `web/src/pages/*.rs`, validated only by e2e in the VM) could be closed by running an **instrumented `jaunder serve`** under a host e2e flow (`LLVM_PROFILE_FILE` set, clean shutdown to flush profraw, merged into the same report). This needs server-binary instrumentation plumbing and a host e2e harness (building on `scripts/e2e-local.sh`), so it is a separate plan, not part of this one.

---

## Self-Review

**Spec coverage:**
- "Improve testing time" (the headline goal) → Phase 0 captures the baseline before any change; Phase 1 Step 3, Phase 3 Step 5, and Phase 4 Task 12 re-measure into one committed before/after table. Every phase is gated on recording its number.
- "Parallel per-test PostgreSQL databases like SQLite" → Phase 1 (Tasks 2-5): template clone + reroute + parallel verification.
- "Does host PostgreSQL make PostgreSQL coverage feasible" → Phase 2 (Tasks 6-7): two-pass merged coverage, baseline regenerated, docs updated.
- "No-VM 99% local solution" → Phase 1 ephemeral script + Phase 4 default tier (no VM).
- "VM-based 100% solution for CI" → Phase 3 consolidated VM + Phase 4 `--full`/CI.
- Resource-constraint / OOM concern → Phase 3 collapses 23×4 GB VMs into one 2 GB VM with bounded `max_connections`.

**Placeholder scan:** Migration-behavior test edits (Phase 1 Task 4, Steps 3-4) are a small, explicitly-listed call-site sweep (lines 364/371/379/413) with the full transformation shown and a `grep` gate (Step 5) before deleting `reset_postgres_schema`; the doc-wording tasks (Phase 2 Task 7, Phase 3 Task 9, Phase 4 Task 11) specify exactly which bullets to change and what they must say. No "TBD"/"handle edge cases" steps.

**Type/name consistency:** `template_postgres_url()` (defined Phase 1 Task 2) is consumed in Tasks 3-4; `ensure_template_db`, `TEMPLATE_DB`, `TEMPLATE_LOCK_KEY` used consistently; `open_existing_database` imported (Task 3 Step 1) before use; `postgresIntegrationCheck`/`postgres-integration`/`postgres-integration-checks` names line up across Phase 3 Steps 1-2; `scripts/with-ephemeral-postgres` referenced identically in Phases 1, 2, 4. Phase 4 Task 10 Step 2 flags the double-wrap risk so coverage isn't wrapped at two layers.

**Known risk carried forward:** the double-wrap of `with-ephemeral-postgres` (internal to `check-coverage` vs. in `verify`) is called out explicitly with the resolution (wrap once, internally).
```

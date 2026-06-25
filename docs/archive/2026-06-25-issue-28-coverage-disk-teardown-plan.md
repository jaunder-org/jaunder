# Per-test Postgres DB Teardown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the `jaunder-coverage` Nix build from exhausting disk by dropping each per-test PostgreSQL database when its test finishes.

**Architecture:** Wrap the existing `TestEnv.base` temp dir in a `TestBase` newtype that `Deref`s to `TempDir` (so the 240 `let TestEnv { state, base }` destructures and all `base.path()` / `&base` uses keep compiling unchanged) and, on `Drop`, issues `DROP DATABASE <clone> WITH (FORCE)`. Peak concurrent per-test databases drops from *total Postgres tests* to *nextest concurrency*.

**Tech Stack:** Rust, sqlx (PostgreSQL), tokio, rstest, cargo-nextest, the `scripts/with-ephemeral-postgres` harness.

## Global Constraints

- **No `Co-Authored-By` trailers** in any commit.
- **Do not commit** without explicit user approval (per repo CLAUDE.md). Steps below include a `git commit`; treat it as *prepare the commit and request approval*, not auto-commit.
- **Per-task gate:** `cargo xtask check --no-test` (clippy + fmt). Final gate: `cargo xtask validate`. Invoke xtask bare (no trailing `; echo`/`| tee`).
- **Worktree cwd caveat:** Serena/context-mode run against the MAIN repo; run gates and the test via `Bash` from the worktree, or with absolute worktree paths.
- This change is **test-only** (`server/tests/**`): no production code, no storage-dialect parity concerns, no coverage-baseline impact on shipped crates.

## Scope notes

- **Acceptance #2 (infra attribution) is already done** (commit `7e98432`): `classify_nextest_output` → `StatusCategory::Infra`, surfaced by the Nix `coverage-gate` and xtask `sentinel_detail`, with tests. This plan adds **no** code for it; the final verification only confirms those tests still pass.
- **`unique_postgres_url()` is out of scope.** It mints a database without a template and is used by only a handful of `postgres_testing_enabled()`-gated tests (`server/tests/misc/commands.rs`, `server/tests/misc/backup_interop.rs`) — a bounded, non-suite-scaling count (~4–6 DBs). The suite-scaling consumer is `template_postgres_url()` via `Backend::setup`. Folding `unique_postgres_url` into the same teardown can be a trivial follow-up if a future run still trips the threshold.

## Reference: current code (for the implementer)

`server/tests/helpers/mod.rs`:
- `pub struct TestEnv { pub state: Arc<AppState>, pub base: TempDir }` (lines ~62–65).
- `impl Backend { pub async fn setup(self) -> TestEnv { ... } }` (lines ~84–101). Postgres arm calls `template_postgres_url().await`, `open_existing_database(&url)`, writes `url.to_string()` to `base/PG_URL_FILE`.
- Helpers available: `postgres_bootstrap_url() -> String`, `quote_postgres_identifier(&str) -> String` (private), `recorded_postgres_url(&TempDir) -> String` (pub), `postgres_testing_enabled() -> bool` (pub).
- `template_postgres_url() -> DbConnectOptions` returns `DbConnectOptions::Postgres { options, .. }`; `options.get_database()` yields the per-test DB name.
- Crate-level `#![allow(clippy::unwrap_used, clippy::expect_used, ...)]` is already present, and `use sqlx::Connection;` is already imported.

`server/tests/misc/main.rs` declares the misc test binary's modules (`mod backup_interop; mod commands; ...`) and includes helpers via `#[path = "../helpers/mod.rs"] mod helpers;`.

---

### Task 1: Per-test Postgres database teardown

**Files:**
- Create: `server/tests/misc/pg_teardown.rs`
- Modify: `server/tests/misc/main.rs` (add `mod pg_teardown;`)
- Modify: `server/tests/helpers/mod.rs` (add `TestBase`, add `drop_test_database`, change `TestEnv.base` type, rewire `Backend::setup`)

**Interfaces:**
- Produces: `pub struct TestBase` with `impl Deref<Target = TempDir>` and `impl Drop`; `TestEnv.base: TestBase` (field name unchanged). Private constructors `TestBase::sqlite(TempDir)` / `TestBase::postgres(TempDir, String)`. Private `fn drop_test_database(db_name: &str)`.
- Consumes: existing `postgres_bootstrap_url`, `quote_postgres_identifier`, `template_postgres_url`, `recorded_postgres_url`, `postgres_testing_enabled`.

- [x] **Step 1: Write the failing regression test**

Create `server/tests/misc/pg_teardown.rs`:

```rust
use crate::helpers::{
    postgres_bootstrap_url, postgres_testing_enabled, recorded_postgres_url, Backend,
};
use sqlx::Connection;

/// Database name (last path segment, query stripped) from a PostgreSQL test URL.
fn db_name_from_url(url: &str) -> String {
    let without_query = url.split('?').next().unwrap_or(url);
    without_query
        .rsplit('/')
        .next()
        .expect("URL has a database segment")
        .to_owned()
}

/// True if `db_name` currently exists in the ephemeral cluster.
async fn database_exists(db_name: &str) -> bool {
    let options: sqlx::postgres::PgConnectOptions =
        postgres_bootstrap_url().parse().expect("bootstrap URL parses");
    let mut conn = sqlx::PgConnection::connect_with(&options)
        .await
        .expect("connect to bootstrap database");
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(db_name)
            .fetch_one(&mut conn)
            .await
            .expect("query pg_database");
    conn.close().await.ok();
    exists
}

#[tokio::test]
async fn per_test_database_is_dropped_on_teardown() {
    if !postgres_testing_enabled() {
        return;
    }

    let env = Backend::Postgres.setup().await;
    let db_name = db_name_from_url(&recorded_postgres_url(&env.base));

    assert!(
        database_exists(&db_name).await,
        "per-test database {db_name} should exist while the TestEnv is alive"
    );

    drop(env);

    assert!(
        !database_exists(&db_name).await,
        "per-test database {db_name} should be dropped once the TestEnv is gone"
    );
}
```

Add the module to `server/tests/misc/main.rs` (keep the list alphabetical):

```rust
mod backup_interop;
mod commands;
mod media_handlers;
mod pg_teardown;
mod static_assets;
```

- [x] **Step 2: Run the test to verify it fails**

Run (from the worktree root):
```
scripts/with-ephemeral-postgres cargo nextest run -p jaunder -E 'test(per_test_database_is_dropped_on_teardown)'
```
Expected: the test **runs** (Postgres is enabled inside the harness) and **FAILS** the second assertion — the per-test database still exists after `drop(env)`, because `base: TempDir` performs no DB teardown. This demonstrates the leak.

- [x] **Step 3: Implement `TestBase`, the drop helper, and rewire `setup`**

In `server/tests/helpers/mod.rs`, change the `TestEnv` definition (the `base` field type only):

```rust
pub struct TestEnv {
    pub state: Arc<AppState>,
    pub base: TestBase,
}

/// Owns a test's temp dir and, on Postgres, the name of the per-test database
/// cloned from the template. Dropping it removes that clone so the ephemeral
/// cluster's data dir does not grow with the suite — the disk-exhaustion fix for
/// issue #28. `Deref`s to the inner `TempDir`, so existing `base.path()` and
/// `&base` uses keep compiling unchanged.
pub struct TestBase {
    dir: TempDir,
    /// `Some(name)` on Postgres; `None` on SQLite.
    postgres_db: Option<String>,
}

impl TestBase {
    fn sqlite(dir: TempDir) -> Self {
        Self {
            dir,
            postgres_db: None,
        }
    }

    fn postgres(dir: TempDir, db_name: String) -> Self {
        Self {
            dir,
            postgres_db: Some(db_name),
        }
    }
}

impl std::ops::Deref for TestBase {
    type Target = TempDir;

    fn deref(&self) -> &Self::Target {
        &self.dir
    }
}

impl Drop for TestBase {
    fn drop(&mut self) {
        if let Some(db_name) = self.postgres_db.take() {
            drop_test_database(&db_name);
        }
    }
}
```

Rewrite `Backend::setup` to build a `TestBase` and capture the per-test DB name:

```rust
impl Backend {
    pub async fn setup(self) -> TestEnv {
        let dir = TempDir::new().unwrap();
        let (state, base) = match self {
            Backend::Sqlite => {
                let state = open_database(&sqlite_url(&dir)).await.unwrap();
                (state, TestBase::sqlite(dir))
            }
            Backend::Postgres => {
                let url = template_postgres_url().await;
                let state = open_existing_database(&url).await.unwrap();
                let DbConnectOptions::Postgres { options, .. } = &url else {
                    unreachable!("template_postgres_url returns Postgres options");
                };
                let db_name = options
                    .get_database()
                    .expect("per-test database URL includes a name")
                    .to_owned();
                // Record the per-test DB URL so raw-SQL helpers reuse this exact
                // database rather than minting a fresh (empty) template clone.
                std::fs::write(dir.path().join(PG_URL_FILE), url.to_string())
                    .expect("write recorded Postgres URL");
                (state, TestBase::postgres(dir, db_name))
            }
        };
        TestEnv { state, base }
    }
}
```

Add the teardown helper near the other Postgres helpers (e.g. just after `unique_postgres_db_name`):

```rust
/// Best-effort `DROP DATABASE <name> WITH (FORCE)` for a per-test clone.
///
/// Runs on a dedicated thread with its own current-thread runtime so it is safe
/// to call from `Drop` regardless of the ambient async context (a fresh thread
/// has no running Tokio runtime, so building one does not panic). The thread is
/// joined before returning, so the clone's disk is reclaimed before the next
/// test allocates. `WITH (FORCE)` (PostgreSQL 13+) terminates any connections
/// still open to the clone, so teardown is robust to drop ordering relative to
/// the `AppState` pool. Errors are swallowed: a failed drop at end-of-run is
/// harmless (the whole cluster is discarded) and panicking in `Drop` during an
/// unwind would abort the process.
fn drop_test_database(db_name: &str) {
    let bootstrap = postgres_bootstrap_url();
    let statement = format!(
        "DROP DATABASE {} WITH (FORCE)",
        quote_postgres_identifier(db_name)
    );
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let Ok(options) = bootstrap.parse::<sqlx::postgres::PgConnectOptions>() else {
                    return;
                };
                if let Ok(mut conn) = sqlx::PgConnection::connect_with(&options).await {
                    let _ = sqlx::query(&statement).execute(&mut conn).await;
                    let _ = conn.close().await;
                }
            });
        });
    });
}
```

- [x] **Step 4: Run the regression test to verify it passes**

Run:
```
scripts/with-ephemeral-postgres cargo nextest run -p jaunder -E 'test(per_test_database_is_dropped_on_teardown)'
```
Expected: **PASS** — the database exists before `drop(env)` and is gone after.

- [x] **Step 5: Verify the whole Postgres test suite still compiles and passes**

This proves the 240 `TestEnv { state, base }` destructures and all `&base` / `base.path()` uses still compile under the `Deref` wrapper, and that teardown doesn't break any test.

Run:
```
scripts/with-ephemeral-postgres cargo nextest run -p jaunder
```
Expected: PASS (no compile errors, no test failures). If a site fails to compile because it consumed `base` by value as a `TempDir`, convert it to use `base.path()` or `&*base`; none were found during planning, so this is a safety net.

- [x] **Step 6: Per-task gate (clippy + fmt)**

Run:
```
cargo xtask check --no-test
```
Expected: exit 0. Read `.xtask/last-result.json` (`jq '.steps'`) only if it fails.

- [x] **Step 7: Prepare the commit (request approval per repo policy)**

```bash
git add server/tests/helpers/mod.rs server/tests/misc/main.rs server/tests/misc/pg_teardown.rs
git commit -m "fix(test-helpers): drop per-test Postgres databases on teardown (#28)

Each Postgres-backed test cloned a per-test database via CREATE DATABASE
... TEMPLATE and never dropped it, so the ephemeral cluster's data dir
grew with the suite and exhausted disk in the jaunder-coverage build
(PG SQLSTATE 53100). Wrap TestEnv.base in a TestBase newtype that drops
the clone (DROP DATABASE ... WITH (FORCE)) on test teardown, bounding
peak databases to nextest concurrency."
```

---

## Final verification (run after Task 1, before ship)

Not a commit task — the landing gate.

- [ ] **Acceptance #2 non-regression:** the existing classification tests still pass:
  `cargo nextest run -p devtool -p coverage -p xtask` (or rely on the full `validate` below). Confirm tests `detects_disk_full_as_infra_even_with_fails`, `category_serializes_kebab_case`, `infra_detail_is_labeled_as_infrastructure` pass.

- [ ] **Optional disk evidence:** after a full Postgres suite run, confirm no leaked clones remain — connect to the bootstrap DB and check `SELECT count(*) FROM pg_database WHERE datname LIKE 'jaunder_test_%'` is ~0 (the template `jaunder_test_template` is expected to remain). The deterministic Task 1 regression test is the primary proof; this is a broader sanity check.

- [ ] **Full gate:** `cargo xtask validate` exits 0.

- [ ] **Conclusive proof:** `Validate` goes green on CI `main` after merge (the disk threshold is a CI-runner property; local disk is too ample to reproduce ENOSPC).

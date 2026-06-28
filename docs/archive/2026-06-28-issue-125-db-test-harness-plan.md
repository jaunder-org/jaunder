# `db-test-harness` Crate Extraction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the both-backend test harness (`Backend`, `TestEnv`, DB provisioning, and the `backends`/`sqlite_only`/`postgres_only` rstest templates) out of `server/tests/helpers/mod.rs` into a dedicated `db-test-harness` workspace crate that both `storage` and `server` build on — behavior-preserving, no committed test-body changes.

**Architecture:** New library crate `db-test-harness` (workspace member) owns the backend-parametrization primitive; it depends on `storage` + `common` and reads `JAUNDER_PG_TEST_URL`. `server/tests/helpers` re-exports from it and keeps only its web/leptos-specific helpers. The crate is a `dev-dependency` of `storage` and `server` (a dev-dep cycle `storage ⇄ db-test-harness`, which Cargo permits).

**Tech Stack:** Rust 2021, `sqlx` (sqlite+postgres), `rstest` 0.26, `rstest_reuse` 0.7, `tokio`, `tempfile`, `chrono`.

## Status: COMPLETE — deviations from the plan as written

- **Task 1 spike** resolved to the **primary path**: `rstest_reuse`'s `#[export]` makes the
  templates `#[macro_export]`-reachable cross-crate; no per-crate shim needed. The spike
  test and its temporary `storage` dev-deps were reverted; Task 1 committed only the crate
  + workspace member.
- **Task 2** gained an unplanned coverage step: the harness is a coverage-measured library
  (unlike the test target it came from), so the env-conditional URL logic was lifted into
  pure, unit-tested helpers (`bootstrap_url`, `splice_db_name`) and only the genuinely-dead
  defensive arms were `// cov:ignore`-marked.
- **Task 3** validation was **stashed as the #126 seed** (not reverted) per a mid-cycle
  decision — so `storage` keeps no committed dependency on the crate in #125; the
  `storage ⇄ db-test-harness` cycle lands with #126 (validated here via the stash).
- **Final review (Opus agent)** fixes: added the `sqlite` sqlx feature so the crate is
  self-sufficient; dropped the unused direct `rstest` dep and the carried-over
  `dead_code`/`unused_macros` allows; corrected ADR-0033/spec dep-cycle tense.

## Global Constraints

- No `Co-Authored-By` trailers in commits.
- Commit messages follow the repo's `type(scope): subject` style, e.g. `refactor(issue-125): …`.
- Per-task gate while iterating: `cargo xtask check --no-test` (clippy + fmt). Final gate: `cargo xtask validate`.
- `db-test-harness` is a **library** crate, not a test target, so it must NOT inherit the workspace `unwrap_used = "deny"` / `expect_used = "deny"` lints. It carries its own `[lints]` + crate-level `#![allow(...)]` mirroring `server/tests/helpers/mod.rs`.
- The throwaway validation conversions in Task 3 are **reverted before any commit** — they are proof of shape, not deliverable.
- Branch: `worktree-issue-125-db-test-harness` (already created). Never commit on `main`.

---

### Task 1: Scaffold `db-test-harness` crate + prove cross-crate rstest templates (the spike)

The one real unknown is whether `rstest_reuse` `#[template]`s defined in a library crate can be `#[apply]`-ed from a *consumer* crate's tests. This task establishes the crate and resolves that strategy before any bulk move.

**Files:**
- Create: `db-test-harness/Cargo.toml`
- Create: `db-test-harness/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members`)
- Modify: `storage/Cargo.toml` (`[dev-dependencies]`)
- Test (throwaway, reverted at end of task): `storage/src/lib.rs` scratch test module

**Interfaces:**
- Produces: crate `db_test_harness` exporting `pub enum Backend { Sqlite, Postgres }` and templates `backends`, `sqlite_only`, `postgres_only` (each `fn(#[case] backend: Backend)`). Later tasks rely on these names.

- [x] **Step 1: Create the crate manifest**

`db-test-harness/Cargo.toml`:

```toml
[package]
name = "db-test-harness"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]
storage.workspace = true
common.workspace = true
chrono = { workspace = true }
rstest = { workspace = true }
rstest_reuse = { workspace = true }
sqlx = { workspace = true, features = ["postgres"] }
tempfile.workspace = true
tokio = { workspace = true }

# NOT `[lints] workspace = true`: this is a library crate (not a test target),
# so the workspace `unwrap_used/expect_used = "deny"` lints must not apply to
# what is deliberately test-support code full of unwrap()/expect().
[lints.clippy]
pedantic = { level = "warn", priority = -1 }
```

- [x] **Step 2: Create `src/lib.rs` with `Backend` + the three templates**

`db-test-harness/src/lib.rs`. NOTE: this is a **library** target compiled without `cfg(test)`, so it trips pedantic doc-lints that clippy suppresses in the test-only `helpers` module — notably `clippy::missing_panics_doc` on the moved `pub` panicking fns (`recorded_postgres_url`, `Backend::setup`, `sqlite_url`, `unique_postgres_url`, `template_postgres_url`, …). The allow block below adds `missing_panics_doc` on top of the `helpers:1-14` set. Treat `cargo xtask check --no-test` (Step 9) as the authority: if clippy reports further pedantic lints (`must_use_candidate`, `doc_markdown`), add them to this block — do NOT assume the helpers set is complete. (The `unwrap_used`/`expect_used` allows are redundant — those are restriction-group, default-allow, and this crate does not enable workspace lints — but kept for parity with `helpers`.)

```rust
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]
#![allow(dead_code)]
#![allow(unused_macros)]

#[allow(unused_imports)]
use rstest::*;
#[allow(clippy::single_component_path_imports)]
use rstest_reuse;
use rstest_reuse::*;

/// The storage backend a test runs against.
#[derive(Copy, Clone)]
pub enum Backend {
    Sqlite,
    Postgres,
}

#[template]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
pub fn sqlite_only(#[case] backend: Backend) {}

#[template]
#[rstest]
#[case::postgres(Backend::Postgres)]
pub fn postgres_only(#[case] backend: Backend) {}

#[template]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
pub fn backends(#[case] backend: Backend) {}
```

- [x] **Step 3: Register the crate in the workspace**

Modify `Cargo.toml` `members` (currently `["common","hydrate","server","storage","web"]`) to add `"db-test-harness"`:

```toml
members = [
  "common",
  "db-test-harness",
  "hydrate",
  "server",
  "storage",
  "web"
]
```

- [x] **Step 4: Add the crate as a dev-dependency of `storage`**

Modify `storage/Cargo.toml` `[dev-dependencies]`:

```toml
[dev-dependencies]
db-test-harness = { path = "../db-test-harness" }
tempfile.workspace = true
tokio = { workspace = true, features = ["macros", "rt"] }
```

- [x] **Step 5: Verify the crate builds**

Run: `cargo build -p db-test-harness`
Expected: PASS (`Finished`).

- [x] **Step 6: Write the throwaway cross-crate spike test (in `storage`)**

Append to `storage/src/lib.rs` a scratch module (deleted in Step 8):

```rust
#[cfg(test)]
mod spike_xcrate_templates {
    use db_test_harness::{backends, Backend};
    use rstest_reuse::apply;

    #[apply(backends)]
    fn template_applies_cross_crate(#[case] backend: Backend) {
        // Compiles + runs only if a crate-exported #[template] can be #[apply]-ed
        // from a different crate. Two cases expected: ::sqlite and ::postgres.
        let _ = matches!(backend, Backend::Sqlite | Backend::Postgres);
    }
}
```

- [x] **Step 7: Run the spike and read the result**

Run: `cargo nextest run -p storage spike_xcrate_templates`
Expected (clean path): two cases run —
`template_applies_cross_crate::sqlite` and `template_applies_cross_crate::postgres` both PASS.

**Decision gate:**
- **Compiles + both cases run** → cross-crate template export works for **direct import** (the `storage`/#126 path). Keep the templates exported from the crate (the design's primary path). Proceed.
  - Caveat: this spike does NOT cover `server`'s path, which is `pub use db_test_harness::backends` re-exported *through* `helpers`, then `#[apply(backends)]` at the call site. Re-exporting a `rstest_reuse` `#[template]` (a name-mangled `macro_rules!`) is a distinct risk. **Task 2 Step 5 (`cargo test -p jaunder --no-run`) is the gate for the re-export path.** If it fails to resolve the re-exported template, switch `helpers` to declare its own local `#[template]`s (the per-crate-shim fallback) while still consuming the shared provisioning core from the crate.
- **Does NOT compile** (`#[apply]` can't resolve the cross-crate template) → fall back to the **per-crate shim**: remove the `#[template]`s from `db-test-harness`; instead export a `pub fn provision_label(Backend) -> &'static str` core and have each consumer crate declare its own local `#[template]`s (a ~10-line block per crate, copied from `helpers`). Record the chosen path in the commit message. All later tasks that say "apply the crate's template" then mean "apply the consumer-local template."

- [x] **Step 8: Remove the throwaway spike test**

Delete the `spike_xcrate_templates` module from `storage/src/lib.rs`. Run `git diff storage/src/lib.rs` and confirm it is empty.

- [x] **Step 9: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: all steps `[ ok ]`, `xtask check PASSED`.

- [x] **Step 10: Commit**

```bash
git add db-test-harness/ Cargo.toml storage/Cargo.toml
git commit -m "refactor(issue-125): scaffold db-test-harness crate with backend templates"
```

(The pre-commit hook runs the full gate; expect it to pass.)

---

### Task 2: Move the harness internals into the crate and re-export from `helpers`

Relocate the backend/provisioning/state helpers into `db-test-harness`, leaving only web/leptos-specific helpers in `server/tests/helpers`. This is the behavior-preserving extraction: the full server suite must stay green on both backends afterward.

**Files:**
- Modify: `db-test-harness/src/lib.rs` (add the moved items)
- Modify: `server/tests/helpers/mod.rs` (remove moved items; re-export)
- Modify: `server/Cargo.toml` (`[dev-dependencies]`)

**Interfaces:**
- Consumes: `Backend`, templates from Task 1.
- Produces (all `pub`, re-exported by `helpers`): `TestEnv`, `TestBase`, `PG_URL_FILE`, `recorded_postgres_url`, `Backend::setup`, `sqlite_url`, `postgres_url`, `postgres_testing_enabled`, `postgres_bootstrap_url`, `postgres_url_string`, `postgres_test_authority`, `nonexistent_postgres_url`, `unique_postgres_url`, `template_postgres_url`, `noop_mailer`, `test_sqlite_state_with_pool`, `seed_posts`.

**Move-set** (cut from `server/tests/helpers/mod.rs` into `db-test-harness/src/lib.rs`), by current line:
- `Backend` impl `setup` (`127-154`) — `Backend` enum itself is already in the crate from Task 1; delete the enum copy from helpers when re-exporting.
- `TestEnv` (`62-65`), `TestBase` (`72-92`) + its `Deref` (`94-100`) and `Drop` (`102-108`) impls.
- `PG_URL_FILE` (`117`), `recorded_postgres_url` (`122-125`).
- Provisioning fns: `sqlite_url` (`236`), `postgres_url` (`242`), `postgres_testing_enabled` (`246`), `postgres_bootstrap_url` (`250`), `postgres_url_string` (`257`), `postgres_url_authority` (`262`, private), `postgres_test_authority` (`277`), `quote_postgres_identifier` (`281`, private), `postgres_url_with_db_name` (`285`, private), `unique_postgres_db_name` (`301`, private), `drop_test_database` (`330`, private), `nonexistent_postgres_url` (`369`), `unique_postgres_url` (`375`), `TEMPLATE_DB`/`TEMPLATE_LOCK_KEY` (`405/410`, private), `ensure_template_db` (`415`, private), `template_postgres_url` (`469`).
- State/seed helpers: `noop_mailer` (`495`), `test_sqlite_state_with_pool` (`501`), `seed_posts` (`541`).

**Stay-set** (remain in `helpers`): `ensure_server_fns_registered` (`172`), `test_options` (`222`), `tmp_storage_path` (`230`), the `websub_capturing` module + `CapturingWebSubClient`.

- [x] **Step 1: Add the moved items to `db-test-harness/src/lib.rs`**

Paste the move-set bodies into the crate. Add the imports they require at crate top (these mirror what `helpers` imports for them):

```rust
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use common::mailer::{MailSender, NoopMailSender};
use sqlx::Connection;
use storage::{
    open_database, open_existing_database, AppState, DbConnectOptions, SqliteAtomicOps,
    SqliteAudienceStorage, SqliteEmailVerificationStorage, SqliteFeedCacheStorage,
    SqliteFeedEventStorage, SqliteInviteStorage, SqliteMediaStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteSubscriptionStorage,
    SqliteUserConfigStorage, SqliteUserStorage,
};
use tempfile::TempDir;
```

Notes for the mover:
- The `sqlx::migrate!("../storage/migrations/sqlite")` call inside `test_sqlite_state_with_pool` resolves relative to the crate manifest dir. `db-test-harness/` is at the same depth as `server/`, so the path string stays `"../storage/migrations/sqlite"` — unchanged.
- If `cargo build -p db-test-harness` reports a missing constructor gated behind `storage`'s `test-utils` feature, add `features = ["test-utils"]` to the `storage` dep (and `common`'s, mirroring `server/Cargo.toml:63-64`). Do not add it speculatively — only if the build demands it.

- [x] **Step 2: Verify the crate builds with the moved code**

Run: `cargo build -p db-test-harness`
Expected: PASS.

- [x] **Step 3: Remove the moved items from `helpers` and re-export**

In `server/tests/helpers/mod.rs`, delete every move-set item (including the `Backend` enum and the three `#[template]`s, now owned by the crate), drop the now-unused `use storage::{…}` / `use common::mailer::{…}` / `rstest`/`rstest_reuse` imports that only those items needed, and add at the top of the module:

```rust
pub use db_test_harness::{
    backends, noop_mailer, nonexistent_postgres_url, postgres_bootstrap_url, postgres_only,
    postgres_test_authority, postgres_testing_enabled, postgres_url, postgres_url_string,
    recorded_postgres_url, seed_posts, sqlite_only, sqlite_url, template_postgres_url,
    test_sqlite_state_with_pool, unique_postgres_url, Backend, TestBase, TestEnv, PG_URL_FILE,
};
```

Keep `ensure_server_fns_registered`, `test_options`, `tmp_storage_path`, and the `websub_capturing` wiring exactly as-is.

- [x] **Step 4: Add the crate as a dev-dependency of `server`**

Modify `server/Cargo.toml` `[dev-dependencies]`, adding:

```toml
db-test-harness = { path = "../db-test-harness" }
```

- [x] **Step 5: Build the server test targets**

Run: `cargo test -p jaunder --no-run`
Expected: PASS (all test binaries link; re-export surface resolves).

- [x] **Step 6: Run the full server suite (SQLite path)**

Run: `cargo nextest run -p jaunder`
Expected: same pass/skip counts as before the move; 0 failures. (Postgres cases skip locally when `JAUNDER_PG_TEST_URL` is unset — that is expected; the both-backend run happens under the coverage gate.)

- [x] **Step 7: Per-task gate**

Run: `cargo xtask check --no-test`
Expected: `xtask check PASSED`.

- [x] **Step 8: Commit**

```bash
git add db-test-harness/src/lib.rs server/tests/helpers/mod.rs server/Cargo.toml
git commit -m "refactor(issue-125): move backend test harness into db-test-harness crate"
```

---

### Task 3: Throwaway dual-crate validation (verify shape, then revert)

Prove the foundation against real call sites in BOTH crates before declaring done. Nothing here is committed.

**Files (all reverted at end):**
- `storage/src/site_config.rs` (one test temporarily parametrized)
- `server/tests/storage/storage.rs` (one test temporarily parametrized)

- [x] **Step 1: Temporarily parametrize one `storage` Tier-2 test**

In `storage/src/site_config.rs`, take the existing `set_and_get_backup_config_round_trips` test and convert it to run on both backends via the crate — replace its `test_pool()` + `SqliteSiteConfigStorage::new` setup with `Backend::setup()` and `state.site_config`, applying `#[apply(db_test_harness::backends)]` (or the consumer-local template if Task 1 chose the shim). Use `db_test_harness::Backend`.

- [x] **Step 2: Temporarily parametrize one `server` test**

In `server/tests/storage/storage.rs`, pick one currently-`#[tokio::test]` SQLite-only test (e.g. `create_user_succeeds_and_get_by_username_returns_record`) and convert it to `#[apply(backends)]` + `backend.setup()` + `state.users`, as the existing `invite_and_atomic_registration_work` (`storage.rs:721`) does.

- [x] **Step 3: Run both under an ephemeral Postgres and confirm BOTH cases execute**

Run (`devtool` lives in the `tools/` workspace, not the root workspace, and its `pg` subcommand needs a `run --` passthrough — `tools/devtool/src/main.rs:38-46`):

```bash
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- \
  cargo nextest run -p storage set_and_get_backup_config_round_trips \
                    -p jaunder create_user_succeeds_and_get_by_username_returns_record
```

(This is the same ephemeral-PG provisioning the coverage gate uses — `tools/devtool/src/coverage/emit.rs:72` calls `pg::with_ephemeral`. If the passthrough misbehaves, the fallback is to run `cargo xtask check` and read the instrumented nextest output under `.xtask/diagnostics/`, but the direct `pg run --` is faster for this probe.)

Expected: **four** test instances run, all PASS:
- `set_and_get_backup_config_round_trips::sqlite`
- `set_and_get_backup_config_round_trips::postgres`  ← the storage Tier-2 test on Postgres
- `create_user_succeeds_and_get_by_username_returns_record::sqlite`
- `create_user_succeeds_and_get_by_username_returns_record::postgres`

**Explicitly confirm the `::postgres` instances are present and did NOT skip.** If the storage `::postgres` case is absent, the "storage tests run under the ephemeral PG pass" assumption is false — STOP and report; issue #126 then needs a coverage-pass change and this finding must be recorded (do not silently proceed).

- [x] **Step 4: Revert both throwaway conversions**

```bash
git checkout -- storage/src/site_config.rs server/tests/storage/storage.rs
git status --short
```

Expected: clean tree (no changes from this task).

---

### Task 4: Final gate

- [x] **Step 1: Run the full CI-faithful gate (no e2e)**

Run: `cargo xtask validate --no-e2e`
Expected: `xtask validate PASSED` — static checks, clippy, coverage all green; the server suite passes on both backends under the coverage pass.

- [x] **Step 2: Confirm clean tree and intended history**

```bash
git status --short
git log --oneline wt-base-issue-125..HEAD
```

Expected: clean tree; **four** commits — two docs commits already on the branch (the spec + ADR-0033 + `docs/README.md` ADR-table row in `4679fd9`; the plan doc) plus the two refactor commits from Tasks 1–2 (scaffold, move). No throwaway test changes present. (The ADR-table row in `docs/README.md` is already committed — no task creates it.)

---

## Notes for the executor

- This is a **refactor**, not feature work: the "test" that must stay green is the existing server suite, plus the throwaway parametrized probes in Task 3. There are no new committed tests.
- If the Task 1 spike chose the per-crate-shim fallback, every "apply the crate's template" instruction means "apply the consumer-local template"; the provisioning core is still shared from the crate.
- Out of scope (downstream issues, do NOT start here): #54 (convert `server/tests/storage/storage.rs` + guard), #126 (storage Tier-2 conversions), #127 (rest of `server/tests` + guard rollout).

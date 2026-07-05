# Issue #135 — Storage dialect tests → dual-backend + dedup + crate-wide guard — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating individual tasks to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax; tick
> them in real time.

**Goal:** Close the Postgres coverage holes in the `storage` crate's dialect-dir
tests by converting backend-agnostic tests to dual-backend and re-homing them to
their generic modules; dedup duplicated pure-logic; and widen the
`test-backend-pattern` guard crate-wide.

**Architecture:** Follow the
[spec](../specs/2026-07-04-issue-135-storage-dialect-test-hygiene.md). Home each
test by _what it proves_ — backend-common (`#[apply(backends)]`) tests go in the
generic home module `storage/src/<trait>.rs` (per #126); decisively
backend-specific tests stay with their dialect code. Conversion uses the
#170/PR#242 harness (`Backend::setup()` + `TestBase::close_pool()`), extended
here with a symmetric `CloseablePool::sqlite()` seed accessor.

**Tech Stack:** Rust, `sqlx` (SQLite + Postgres), `rstest`/`rstest_reuse`
(`#[apply(...)]` templates in `storage::test_support`), `cargo nextest`,
`cargo xtask` (fmt + clippy + Nix coverage).

## Global Constraints

- **No `Co-Authored-By` trailer** on commits. One clean commit per task.
- **Per-commit gate:** the pre-commit hook runs the full `cargo xtask check`
  (fmt + clippy `-D warnings` + Nix coverage incl. ephemeral Postgres). Run
  `cargo xtask check` (via `devtool run -- cargo xtask check`) green **before**
  committing (**jaunder-commit**).
- **Coverage is a single workspace-wide instrumented nextest pass with ephemeral
  Postgres live** (`tools/devtool/src/coverage/emit.rs:74-83`) —
  placement-independent; an `#[apply(backends)]` test is covered on both
  backends wherever it lives. **Coverage must not lower** (ADR-0050 stateless
  gate: `cov:ignore` + CRAP threshold).
- **Guard (`xtask/src/steps/test_pattern_check.rs`):** every `#[tokio::test]`
  under the scanned roots must carry
  `#[apply(backends|backends_matrix|sqlite_only|postgres_only)]` or a
  `// guard:no-backend — <reason>` marker. Sync `#[test]` is exempt. It is
  widened to `storage/src` in Task 14 (the **last** code task) — until then,
  intermediate commits stay green because the guard still scans only
  `server/tests`.
- **Backend templates** live in `storage/src/test_support.rs` and are used
  in-crate via
  `use crate::test_support::{backends, sqlite_only, postgres_only, Backend, TestEnv};`.
- **Conversion template** (the canonical rewrite, applied by every convert
  task):

  ```rust
  // BEFORE — in storage/src/sqlite/<trait>.rs, single-backend:
  #[tokio::test]
  async fn <name>() {
      let pool = sqlite_pool().await;
      let storage = Sqlite<Trait>Storage::new(pool.clone());
      pool.close().await;                       // (fault-injection variant)
      let result = storage.<method>(...).await;
      assert!(result.is_err());
  }

  // AFTER — moved into storage/src/<trait>.rs's #[cfg(test)] mod tests, dual-backend:
  #[apply(backends)]
  #[tokio::test]
  async fn <name>(#[case] backend: Backend) {
      let TestEnv { state, base } = backend.setup().await;
      base.close_pool().await;                  // fault via the harness
      let result = state.<handle>.<method>(...).await;   // reach handle from AppState
      assert!(result.is_err());
  }
  ```

  Each home module's `#[cfg(test)] mod tests` **must import the rstest macros**
  exactly as #126's exemplar (`storage/src/site_config.rs:312`) does —
  `#[apply(backends)]` expands `#[rstest]`/`#[case]` onto the fn, which need
  `rstest` in scope:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use rstest::*;
      use rstest_reuse::*;
      use crate::test_support::{backends, Backend, TestEnv};
      // + trait/record imports the tests use, e.g. `use crate::{MediaRecord, MediaStorage as _};`
      // ... converted #[apply(backends)] tests ...
  }
  ```

  For **data-injection** variants, seed bad rows via the agnostic
  `base.pool().execute(sql)` (added in Task 12b) with per-backend SQL, instead
  of `close_pool()`. For **behavioral** variants (e.g. feed_cache CRUD),
  seed/act/assert through `state.<handle>` on both backends.

---

## Task 1: File the separable follow-up issues

**Files:** none (GitHub issues via **jaunder-issues**).

- [x] **Step 1:** Filed
      **[#257](https://github.com/jaunder-org/jaunder/issues/257)** — "storage:
      move SmtpConfig/SmtpTlsMode type defs out of the storage crate" (Task;
      added to Jaunder Backlog #1).
- [x] **Step 2:** Filed
      **[#258](https://github.com/jaunder-org/jaunder/issues/258)** —
      "storage/backup: unify INSERT placeholder token (?n → $n) across dialects"
      (Task; added to Jaunder Backlog #1).
- [x] **Step 3:** Issue numbers #257/#258 recorded here; referenced in Task 15's
      ADR cross-refs. No commit (issues only).

---

## Task 2: Verify-first audit → per-test classification table

**Files:** Create `docs/superpowers/plans/2026-07-05-issue-135-audit.md` (the
table; not code).

**Interfaces:**

- Produces: a table consumed by Tasks 7–13 (convert list + target home module)
  and Task 13 (decisive keeps).

- [ ] **Step 1:** Enumerate every async `#[tokio::test]` under
      `storage/src/sqlite/*.rs`, `storage/src/postgres/*.rs`, and
      `storage/src/test_support.rs`. (Reference inventory: 49 dialect + 2
      harness; see spec.)
- [ ] **Step 2:** For each, resolve the `state.<handle>` it exercises to its
      **home module** —
      `rg -n 'trait \w+Storage|struct \w+Store' storage/src/*.rs` maps
      handle→module (e.g. `media`→`media.rs`, `feed_cache`→`feed_cache.rs`;
      `password_resets`/`email_verifications` may home to
      `password.rs`/`email.rs` — record the actual file).
- [ ] **Step 3:** Classify each: **convert** (default — backend-agnostic; record
      target home module + fault mechanism: close-pool / data-injection /
      behavioral), **dedupe-delete** (already dual-covered in
      `server/tests/storage/storage.rs` — cite the covering test; or a redundant
      PG constructor smoke test), or **decisive-keep** (record the _decisive_
      backend-exclusive reason — backend-only syntax/feature). A keep without a
      decisive reason is not allowed.
- [ ] **Step 4:** Explicitly resolve the known-tricky ones: the 3 raw-SQL
      data-injection tests (`sqlite/users.rs:17,53,74` — `blocked_update` may
      keep if no non-contrived PG trigger equivalent); the 2 PG constructor
      tests (`postgres/mod.rs:380,443` → delete) vs the 1 methods-error test
      (`:455` → convert); `sqlite/backup.rs`'s 5 async (decisive `sqlite_only`);
      `test_support.rs`'s 2 (`seed_user`→convert,
      type-guard→`guard:no-backend`);
      `set_password`/`test_session_record_from_row` (dedupe vs storage.rs).
- [ ] **Step 5: CHECKPOINT** — present the table to the user for confirmation
      before executing conversions (Tasks 7+). Commit the audit doc:
      `git commit -m "docs(issue-135): dialect-test classification audit"`.

---

## Task 3: Convert the `seed_user` harness smoke test to dual-backend

**Files:**

- Modify: `storage/src/test_support.rs` (its `#[cfg(test)] mod tests`,
  ~:615-670)

Rationale: spec says convert `seed_user_creates_a_user` (`test_support.rs:620`)
to `#[apply(backends)]` (it's a backend-agnostic harness smoke test). If left as
a bare `#[tokio::test]` it fails the widened guard in Task 14. (The
`CloseablePool::sqlite()` accessor is added in Task 12b, together with its first
consumers, so its happy arm is covered in the same commit — adding it here with
no consumer would fail the ADR-0050 coverage gate.)

- [ ] **Step 1: Rewrite** `seed_user_creates_a_user` to the template. Add
      `use rstest::*; use rstest_reuse::*;` and `TestEnv`/`backends` to the test
      module's imports (it already `use super::{…, Backend}`):

```rust
#[apply(backends)]
#[tokio::test]
async fn seed_user_creates_a_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let id = seed_user(&env.state).await;
    assert!(id > 0);
}
```

- [ ] **Step 2: Run, verify:**
      `devtool run -- cargo nextest run -p storage seed_user_creates_a_user` →
      both `::sqlite` and `::postgres` cases pass.
- [ ] **Step 3: Commit:** `devtool run -- cargo xtask check` green;
      `git commit -m "test-support(#135): run seed_user smoke test on both backends"`.

---

## Task 4: Dedup SQL quoting into `storage/src/sql.rs`

**Files:**

- Create: `storage/src/sql.rs`
- Modify: `storage/src/lib.rs` (add `pub(crate) mod sql;`),
  `storage/src/sqlite/backup.rs`, `storage/src/postgres/backup.rs`,
  `storage/src/postgres/bootstrap.rs`, `storage/src/test_support.rs`

**Interfaces:**

- Produces: `pub(crate) fn quote_identifier(&str) -> String`,
  `pub(crate) fn quote_literal(&str) -> String` in `crate::sql`.

- [ ] **Step 1: Write the tests** in `sql.rs`'s `#[cfg(test)] mod tests` (moved
      from the dialect copies — identifier _and_ literal):

```rust
#[test]
fn quote_identifier_wraps_and_escapes_double_quotes() {
    assert_eq!(quote_identifier("users"), "\"users\"");
    assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
}

#[test]
fn quote_literal_wraps_and_escapes_single_quotes() {
    assert_eq!(quote_literal("password"), "'password'");
    assert_eq!(quote_literal("can't"), "'can''t'");
}
```

- [ ] **Step 2: Run, verify fail:**
      `devtool run -- cargo nextest run -p storage quote_` → FAIL (`crate::sql`
      not defined).
- [ ] **Step 3: Implement** `storage/src/sql.rs`:

```rust
//! Shared SQL-string helpers used by both dialects' assembled (non-placeholder) SQL.

/// SQL-standard identifier quoting: wrap in double quotes, doubling any interior `"`.
pub(crate) fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// SQL-standard literal quoting: wrap in single quotes, doubling any interior `'`.
pub(crate) fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
```

Add `pub(crate) mod sql;` to `storage/src/lib.rs`.

- [ ] **Step 4: Repoint + delete copies:** in `sqlite/backup.rs`,
      `postgres/backup.rs`, `postgres/bootstrap.rs`, `test_support.rs`, replace
      the local
      `quote_identifier`/`quote_literal`/`quote_postgres_identifier`/`quote_postgres_literal`
      definitions and their call sites with `crate::sql::quote_identifier` /
      `crate::sql::quote_literal` (bootstrap's `order_by_clause` param and
      `backup.rs:243` already take `fn(&str)->String` — pass
      `crate::sql::quote_identifier`). Delete the moved dialect quoting tests.
      Verify one definition each:
      `rg -n 'fn quote_identifier|fn quote_literal|fn quote_postgres_' storage/src`
      → only `sql.rs`.
- [ ] **Step 5: Run + verify:** `devtool run -- cargo nextest run -p storage` →
      PASS; `devtool run -- cargo xtask check` → green (no `dead_code`).
- [ ] **Step 6: Commit:**
      `git commit -m "storage(#135): dedup SQL quoting helpers into storage::sql"`.

---

## Task 5: Dedup `parse_status` into `storage/src/feed_events.rs`

**Files:**

- Modify: `storage/src/feed_events.rs` (add `parse_status` + its test next to
  `FeedEventStatus` at :13), `storage/src/sqlite/feed_events.rs` (delete copy +
  test, repoint), `storage/src/postgres/feed_events.rs` (delete copy incl. its
  `// cov:ignore` markers, repoint)

**Interfaces:**

- Produces: `pub(crate) fn parse_status(&str) -> FeedEventStatus` in
  `crate::feed_events`.

- [ ] **Step 1: Write the test** (moved from `sqlite/feed_events.rs:154`) in
      `feed_events.rs`'s test module:

```rust
#[test]
fn parse_status_handles_all_statuses() {
    assert_eq!(parse_status("pending"), FeedEventStatus::Pending);
    assert_eq!(parse_status("claimed"), FeedEventStatus::Claimed);
    assert_eq!(parse_status("done"), FeedEventStatus::Done);
    assert_eq!(parse_status("failed"), FeedEventStatus::Failed);
    assert_eq!(parse_status("???"), FeedEventStatus::Failed); // defensive fallback
}
```

- [ ] **Step 2: Run, verify fail:**
      `devtool run -- cargo nextest run -p storage parse_status_handles_all_statuses`
      → FAIL / wrong location.
- [ ] **Step 3: Implement** — add to `storage/src/feed_events.rs` next to
      `FeedEventStatus`:

```rust
pub(crate) fn parse_status(s: &str) -> FeedEventStatus {
    match s {
        "pending" => FeedEventStatus::Pending,
        "claimed" => FeedEventStatus::Claimed,
        "done" => FeedEventStatus::Done,
        _ => FeedEventStatus::Failed,
    }
}
```

Repoint `sqlite/feed_events.rs:47`-style and `postgres/feed_events.rs:47` call
sites to `crate::feed_events::parse_status`; delete both dialect copies
**including the PG copy's `// cov:ignore-start/stop`** markers.

- [ ] **Step 4: Run + verify:**
      `devtool run -- cargo nextest run -p storage parse_status` → PASS;
      `devtool run -- cargo xtask check` green — confirm coverage not lowered
      (the all-arms test covers the single merged fn; `cov:ignore` removal is
      safe).
- [ ] **Step 5: Commit:**
      `git commit -m "storage(#135): dedup parse_status into feed_events, drop PG cov:ignore dup"`.

---

## Task 6: Relocate `record_from_row` tests into `storage/src/helpers.rs`

**Files:**

- Modify: `storage/src/helpers.rs` (test module ~:718-808),
  `storage/src/postgres/mod.rs` (delete the 3 `test_*_record_from_row` at
  :386,411,431)

- [ ] **Step 1: Write the moved tests** — into `helpers.rs`'s test module, port
      the **`Some`-branch** cases from `postgres/mod.rs` (which populate `Some`
      display_name/bio/email + `email_verified=true` for user, and `Some`
      `used_at`/`used_by` for invite). Keep the exact assertions from
      `postgres/mod.rs:386-408` (`test_user_record_from_row`) and `:431-440`
      (`test_invite_record_from_row`), renamed to avoid collision (e.g.
      `user_record_from_row_maps_some_fields`,
      `invite_record_from_row_maps_some_fields`). Do **not** port
      `test_session_record_from_row` (redundant with helpers.rs's existing
      `session_and_invite_row_helpers_round_trip`).
- [ ] **Step 2: Run, verify fail** (if names new) or **delete-then-verify:**
      delete the 3 tests from `postgres/mod.rs`;
      `devtool run -- cargo nextest run -p storage record_from_row` → the moved
      `Some`-branch tests run in helpers.rs.
- [ ] **Step 3:** No production code changes (functions already `pub(crate)` in
      helpers.rs).
- [ ] **Step 4: Verify:** `devtool run -- cargo xtask check` green — coverage of
      `build_user_record`/`build_invite_record` `Some` branches preserved
      (moved, not dropped).
- [ ] **Step 5: Commit:**
      `git commit -m "storage(#135): relocate record_from_row Some-branch tests into helpers"`.

---

## Tasks 7–12: Convert + re-home the backend-agnostic dialect tests (per home module)

Each task below follows the **Conversion template** (Global Constraints):
**move** the tests out of the dialect file into the target home module's
`#[cfg(test)] mod tests` (create the block +
`use crate::test_support::{backends, Backend, TestEnv};` and the trait/record
imports if absent), rewrite to `#[apply(backends)]`, delete the emptied dialect
`#[cfg(test)] mod tests` and any now-dead `pool()` helper in that file. The
**audit table (Task 2) is authoritative** for the exact per-test list and any
per-test wrinkle; the lists below are the expected set to confirm against it.

### Task 7: `media` (exemplar — fully worked)

**Files:** Modify `storage/src/media.rs` (add/extend test module), delete
`#[cfg(test)] mod tests` in `storage/src/sqlite/media.rs`.

Expected converts (all close-pool): `create_media`, `get_media`, `list_media`,
`delete_media`, `get_user_upload_usage`, `find_by_hash`
`_with_closed_pool_returns_error` (`sqlite/media.rs:53-118`).

- [ ] **Step 1: Write the converted tests** in `storage/src/media.rs`'s test
      module. Example (the other five follow identically, varying method +
      assertion):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;
    use rstest_reuse::*;
    use crate::test_support::{backends, Backend, TestEnv};
    use crate::{MediaRecord, MediaSource, MediaStorage as _};

    #[apply(backends)]
    #[tokio::test]
    async fn create_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        let record = MediaRecord {
            user_id: 1, sha256: "abc123".into(), filename: "test.jpg".into(),
            source: MediaSource::Upload, content_type: "image/jpeg".into(),
            size_bytes: 1024, source_url: None, created_at: chrono::Utc::now(),
        };
        assert!(state.media.create_media(&record).await.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn delete_media_with_closed_pool_returns_error(#[case] backend: Backend) {
        let TestEnv { state, base } = backend.setup().await;
        base.close_pool().await;
        assert!(matches!(
            state.media.delete_media(1, "abc123", "test.jpg", &MediaSource::Upload).await,
            Err(DeleteMediaError::Internal(_))
        ));
    }
    // get_media, list_media, get_user_upload_usage, find_by_hash: same shape, `.is_err()`.
}
```

- [ ] **Step 2: Delete** the entire `#[cfg(test)] mod tests` from
      `storage/src/sqlite/media.rs`.
- [ ] **Step 3: Run, verify:**
      `devtool run -- cargo nextest run -p storage media` → the `::sqlite` and
      `::postgres` cases both run and pass.
- [ ] **Step 4: Gate + commit:** `devtool run -- cargo xtask check` green;
      `git commit -m "storage(#135): convert media closed-pool tests to dual-backend, re-home to media.rs"`.

### Task 8: `posts` — Modify `storage/src/posts.rs` (already has a #126 test module at :1837), delete converted tests from `sqlite/posts.rs`. Expected: `create_post`/`get_post_by_id`/`list_published` `_with_closed_pool` (3), `tag_post_insert_error_returns_internal`, `list_collection_by_user_orders...` (behavioral — seed via `state.posts`, assert order/exclusion on both). Follow the template.

### Task 9: `invites` → `storage/src/invites.rs`; `sessions` → `storage/src/sessions.rs`. Expected: invites `create`/`use`/`list` `_with_closed_pool` (3); sessions `authenticate_with_closed_pool` (1). (SQLite-specific `touch_and_load` divergence, if any test asserts it, is a decisive keep — Task 13.) Follow the template.

### Task 10: `password_resets` + `email_verifications` → their home modules (per audit: likely `password.rs` / `email.rs`). Expected: `create`/`use` `_with_closed_pool` (2 each). Follow the template.

### Task 11: `feed_cache` → `storage/src/feed_cache.rs` (behavioral CRUD); `feed_events` → `storage/src/feed_events.rs` (enqueue/claim/mark behavioral). Expected: feed_cache `upsert_then_get`, `second_upsert_updates`, `get_missing`, `delete_removes` (4); the `sqlite/feed_events.rs` async behavioral tests (~8, seed/act/assert through `state.feed_events` on both backends). Remove the dead `pool()` helpers in both dialect files.

### Task 12a: `users` no-SQL error paths + `atomic` + delete redundant PG constructor tests

Convert (via the template, no raw SQL): users `create_user_with_hash_error`,
`authenticate_with_verify_error`, `authenticate_with_corrupted_hash`
(magic-password/hash paths that force errors without seeding) → users home
module; `atomic` `create_user_with_invite_{hash_failure,insert_error}` (2) via
`state.atomic` → `storage/src/atomic.rs`;
`postgres/mod.rs:455 test_storage_methods_with_lazy_pool_cover_error_paths` (via
`state`) → its home. **Delete** the 2 redundant PG constructor tests
`postgres/mod.rs:380,443` (already covered by every PG `setup()`). Delete
emptied dialect test modules. One commit.

### Task 12b: add agnostic `CloseablePool::execute()` + convert `users` raw-SQL data-injection + remove `sqlite_pool()`

**Interfaces produced:**
`pub async fn CloseablePool::execute(&self, sql: &str) -> Result<(), sqlx::Error>`
— the seed counterpart to `close()`, dispatching on the enum internally so
callers stay backend-agnostic (no typed accessor, no panic arm). Added **here**
with its consumers, so both match arms are covered by the dual-backend
data-injection tests in the same commit (ADR-0050 fails on any uncovered line).
Do **not** add a typed `sqlite()` accessor — that would perpetuate the
`postgres()` asymmetry; the agnostic `execute()` mirrors `close()`.
(`postgres()` stays for pre-existing typed _inspect_ tests, out of scope.)

First, add to `impl CloseablePool` in `test_support.rs`:

```rust
/// Runs a raw statement against whichever backend this env uses — the seed
/// counterpart to [`close`](CloseablePool::close), dispatched internally so
/// callers stay backend-agnostic. (The SQL string may still be dialect-specific.)
pub async fn execute(&self, sql: &str) -> Result<(), sqlx::Error> {
    match self {
        CloseablePool::Sqlite(pool) => { sqlx::query(sql).execute(pool).await?; }
        CloseablePool::Postgres(pool) => { sqlx::query(sql).execute(pool).await?; }
    }
    Ok(())
}
```

Both arms are exercised by the `#[apply(backends)]` data-injection tests below
(each runs on both backends), so no dedicated coverage test is needed. Then
convert the raw-SQL data-injection tests
(`authenticate_with_invalid_email_in_db`, `authenticate_with_corrupted_hash` if
it seeds a row, and — if the audit rules it convertible —
`authenticate_with_blocked_update`) via `base.pool().execute(sql)` with
per-backend seed SQL:

```rust
#[apply(backends)]
#[tokio::test]
async fn authenticate_with_invalid_email_in_db_returns_internal_error(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    // per-backend seed SQL (exact columns per the real users schema — confirm in iterate)
    let sql = match backend {
        Backend::Sqlite => "INSERT INTO users (username, password_hash, email, created_at) \
                            VALUES ('u', 'x', 'not-an-email', datetime('now'))",
        Backend::Postgres => "INSERT INTO users (username, password_hash, email, created_at) \
                             VALUES ('u', 'x', 'not-an-email', now())",
    };
    base.pool().execute(sql).await.unwrap();   // agnostic; dispatches on the enum
    assert!(state.users.authenticate("u", &"password123".parse().unwrap()).await.is_err());
}
```

`blocked_update` converts (audit-confirmed) with an equivalent PG
update-blocking trigger (`CREATE FUNCTION … RAISE EXCEPTION` +
`CREATE TRIGGER … BEFORE UPDATE OF last_authenticated_at`). **Also delete the
now-superseded PG-only twin `server/tests/storage/storage.rs:892`**
(`authenticate_with_corrupted_hash`'s `postgres_only` copy) — the converted
dialect test's Postgres arm supersedes it (audit note 3); leaving it would
double-cover the PG arm. **This task's last consumer of `sqlite_pool()`
(`storage/src/sqlite/mod.rs:323`) dies here — delete `sqlite_pool()`** (else
clippy `dead_code` fails; the 5 `sqlite/backup.rs` decisive-keeps build their
own `SqliteConnection` and don't use it). One commit.

Each of Tasks 8–12b: run `devtool run -- cargo nextest run -p storage <area>`,
gate with `cargo xtask check`, one commit per task.

---

## Task 13: Annotate the decisive-keep residue + non-DB root tests + backup.rs

**Files:** Modify `storage/src/postgres/bootstrap.rs`,
`storage/src/sqlite/backup.rs`, `storage/src/smtp.rs`, `storage/src/db.rs`,
`storage/src/helpers.rs`, `storage/src/test_support.rs`,
`storage/src/backup.rs`, plus any decisive-keep dialect test the audit flagged.

- [ ] **Step 1:** On each decisive-keep async test, add `#[apply(sqlite_only)]`
      or `#[apply(postgres_only)]` + `#[case] backend: Backend` (with
      `let _ = backend;`) + a **decisive** `// reason:` line:
      `postgres/bootstrap.rs` admin-connection test
      (`CREATE ROLE`/`CREATE DATABASE`); the 5 `sqlite/backup.rs` async tests
      (`PRAGMA`/`sqlite_master`); `blocked_update` if kept.
- [ ] **Step 2:** On the 13 non-DB root async tests (`smtp.rs` 7 mock-store,
      `db.rs` 3 URL-routing, `helpers.rs` 3 hashing) + `test_support.rs`'s
      `postgres_accessor_rejects_a_sqlite_pool` (a bare `#[tokio::test]`), add
      `// guard:no-backend — <reason>`. (The new
      `sqlite_accessor_rejects_a_postgres_pool` from Task 12b is a sync
      `#[test]` → already guard-exempt.)
- [ ] **Step 3:** On root `backup.rs`'s 3 orchestration tests, add
      `#[apply(sqlite_only)]` +
      `// reason: backup export/restore SQL is backend-specific (ADR-0019); Postgres orchestration coverage tracked in #136`.
- [ ] **Step 4:** `devtool run -- cargo xtask check` green.
- [ ] **Step 5: Commit:**
      `git commit -m "storage(#135): annotate decisive single-backend + non-DB tests for the guard"`.

---

## Task 14: Widen `test-backend-pattern` to `storage/src` (xtask)

**Files:** Modify `xtask/src/steps/test_pattern_check.rs`

**Interfaces:**

- Consumes: existing pure `problems(&[(String,String)]) -> Option<String>`,
  `rust_files()`.
- Produces: `run()` scanning `["server/tests", "storage/src"]`.

- [ ] **Step 1: Write the failing test** — add to the module's
      `#[cfg(test)] mod tests`, proving a storage-dialect-pathed bare tokio test
      is flagged and annotated ones are clean (the pure `problems()` already
      handles content; this locks the storage path + multi-root intent):

```rust
#[test]
fn storage_dialect_bare_tokio_test_is_flagged() {
    let scanned = vec![("storage/src/sqlite/foo.rs".to_string(), BARE.to_string())];
    assert!(problems(&scanned).is_some());
}

#[test]
fn storage_root_list_includes_storage_src() {
    assert!(TEST_ROOTS.contains(&"storage/src"));
}
```

- [ ] **Step 2: Run, verify fail:**
      `devtool run -- cargo nextest run -p xtask test_pattern` → FAIL
      (`TEST_ROOTS` not defined).
- [ ] **Step 3: Implement** — replace `const TEST_ROOT: &str = "server/tests";`
      with `const TEST_ROOTS: &[&str] = &["server/tests", "storage/src"];`;
      rewrite `run()` to accumulate `rust_files()` across every root (hard-fail
      if **any** root is missing), then call `problems()` on the union; update
      the module-doc header (drop "only `server/tests`"). Keep the single
      `test-backend-pattern` step.
- [ ] **Step 4: Run, verify pass:**
      `devtool run -- cargo nextest run -p xtask test_pattern` → PASS. Then the
      real gate: `devtool run -- cargo xtask check` → green (proves every
      `storage/src` tokio test is now annotated — this is the milestone's
      enforcement going live).
- [ ] **Step 4b: Verify AC1a (re-homing) directly** —
      `rg -c 'apply\(backends\)' storage/src/sqlite storage/src/postgres`
      returns **0** matches (the guard alone won't catch a dual-backend test
      wrongly left in a dialect file; this check does). If nonzero, that test
      was not re-homed — fix before committing.
- [ ] **Step 5: Commit:**
      `git commit -m "xtask(#135): widen test-backend-pattern guard to storage/src"`.

---

## Task 15: ADR — record the storage-test convention

**Files:** Create `docs/adr/drafts/storage-test-homing-and-dual-backend.md`
(numberless draft, via **jaunder-adr**).

- [ ] **Step 1:** Draft the ADR per spec Component 6: _home a test by what it
      proves_ (backend-common → generic home module; decisively backend-specific
      → with its dialect code); _presume a coverage gap_ and convert
      backend-agnostic storage tests to both backends via the fault/seed
      harness; _classify pure-logic by generalizability_ and dedup. State that
      it **supersedes** the stale "dialect files carry no in-file tests (for
      coverage reasons)" belief — coverage is now a single workspace-wide
      PG-live pass (`CONTRIBUTING.md:428-431`), placement-independent.
      Cross-reference ADR-0019, #126, #170, and the two follow-up issues from
      Task 1.
- [ ] **Step 2:** `devtool run -- cargo xtask check` green.
- [ ] **Step 3: Commit:**
      `git commit -m "docs(adr): draft storage test-homing + dual-backend convention (#135)"`.
- [ ] **Step 4: Final acceptance (AC10)** — run the full local gate
      `devtool run -- cargo xtask validate` (static + coverage + e2e; long/cold
      → Bash background mode) → green with no coverage lowering. This is the
      "green → done" proof before ship.

---

## Self-review notes

- **Spec coverage:** Task 1→Separable concerns; Task 2→verify-first audit; Task
  3→harness `sqlite()`; Tasks 4-6→pure-logic dedup (Components 2); Tasks
  7-12→convert+re-home (Component 3, incl. AC 1/1a); Task 13→decisive-keep +
  non-DB + backup annotation (Component 4/5); Task 14→guard (Component 5, AC 7);
  Task 15→ADR (Component 6, AC 8). AC 3 (`sqlite()`)→Task 3; AC 8 (dead
  scaffolding)→Tasks 4/11/12.
- **Ordering:** dedup + harness (3-6) are audit-independent and can precede or
  interleave; conversions (7-12) consume the audit; the guard (14) is **last**
  so intermediate commits stay green. ADR (15) last.
- **Risk:** exact home-module targets and the convert/keep/delete split are the
  audit's job (Task 2 checkpoint) — the conversion tasks are templated and
  confirm against it, not guesses.

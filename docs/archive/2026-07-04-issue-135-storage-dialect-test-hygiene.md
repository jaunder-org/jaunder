# Design — Issue #135: storage dialect tests → dual-backend + dedup + crate-wide guard

- Date: 2026-07-04
- Issue: [#135](https://github.com/jaunder-org/jaunder/issues/135)
- Status: draft (awaiting approval) — revised after two cold reviews + adding
  the test **re-homing** dimension (convert-and-move to the generic home module,
  per #126; the stale "dialect files carry no in-file tests" coverage rationale
  is superseded — coverage is now placement-independent).
- Builds on **#170 / PR #242** (merged 2026-07-04): the backend-generic
  fault-injection harness (`CloseablePool`/`TestBase::close_pool`) and its
  `#[apply(backends)]` conversion template. Follows #126 (root-file dual-backend
  contract tests) and #54/#127 (the `test-backend-pattern` guard).

## Context / Problem

Milestone #5's charter: _every test that asserts backend-common behavior runs on
both SQLite and Postgres._ #126 converted the backend-common tests in the _root_
storage files; the per-backend **dialect-dir** tests (`storage/src/sqlite/*.rs`,
`storage/src/postgres/*.rs`) were left — **49 async `#[tokio::test]`** + **20
sync `#[test]`** (pure-logic). Most of the async ones assert backend-common
behavior on SQLite only: Postgres coverage gaps left from before the #242
harness existed.

### Governing lens: presume a coverage gap; keep single-backend only on decisive grounds

A test is not dialect-specific merely because it lives in a dialect file, binds
a private helper, was written against SQLite, or _injects its fault via SQLite
SQL_. **Presumption: a single-backend storage test is a Postgres coverage gap
and must be converted to run on both backends.** The burden is on _keeping_ a
test single-backend, met only by a **decisive, non-spurious backend-exclusive**
reason: syntax/feature only one backend supports — e.g. Postgres
`CREATE ROLE`/`CREATE DATABASE` DDL, PG SQLSTATE codes, PG
`CAST($n AS type)`/`OVERRIDING SYSTEM VALUE`, SQLite `PRAGMA`/`sqlite_master`
introspection, `json1`, or a harness type-guard tied to one pool variant. **Not
decisive:** "looks single-backend," "error path," "lazy/closed pool," or _"the
seed SQL is written in one dialect"_ — the behavior is agnostic; provide
per-backend injection SQL and convert. When in doubt, convert.

### Two enabling facts

1. **The fault is backend-agnostic and the harness now supports both backends.**
   `test_support.rs:53` documents pool-close as _"the backend-agnostic
   storage-error-propagation fault… identical on SQLite and Postgres,"_ and #242
   built `TestBase::close_pool()` + `CloseablePool` (both `Sqlite`/`Postgres`
   arms) to inject it on either. The dialect closed-pool tests still use a bare
   `sqlite_pool()` (`media.rs:54`) only because they predate that harness.
2. **`AppState` exposes every storage handle** a converted test needs
   (`app_state.rs:25`:
   `site_config, users, sessions, invites, atomic, email_verifications, password_resets, posts, media, user_config, feed_cache, feed_events`,
   …) as `Arc<dyn …>`, reachable via `&*state.<handle>`. Verified.
3. **Coverage is placement-independent.** The gate runs one workspace-wide
   instrumented nextest pass with an ephemeral Postgres live for the whole run
   (`tools/devtool/src/coverage/emit.rs:74-83`; `CONTRIBUTING.md:428-431`) — no
   `#[ignore]`, no `-p jaunder`/`--run-ignored` split. An `#[apply(backends)]`
   test gets _both_ backends instrumented wherever it lives. (This refutes an
   older two-pass belief — now eradicated — that only `server/tests` got PG
   coverage; re-homing is therefore an organizational choice, not a coverage
   one.)

### Homing: by what the test proves, not which backend runs it

A dialect file (`storage/src/sqlite/media.rs`) holds one backend's _divergent_
impl (`impl MediaDialect for Sqlite`); the generic `MediaStore<DB>` + the
`MediaStorage` trait live in the home module `storage/src/media.rs`. A
now-common `#[apply(backends)]` test proves the **generic** contract, so it
belongs in the **home module's** test block — not the backend-specific dialect
file, where a dual-backend test is self-contradictory. This is exactly #126's
established pattern: all 47 of its converted tests live in-file in the home
modules (`site_config.rs:311`, `auth.rs:102`, …) and **zero** in any dialect
file. So converting a dialect test is \*convert **and re-home\*** to
`storage/src/<trait>.rs`. Only a **decisively backend-specific** test stays with
its dialect code — because that code (SQLite backup PRAGMA introspection, PG
bootstrap DDL) _is_ what it proves and has no generic home. After #135, a
dialect file's `#[cfg(test)]` block holds only such tests, or is deleted when
none remain.

### Classification of the 49 async dialect tests (verified per-file)

- **Convert → `#[apply(backends)]` (default; ~40):**
  - Closed-pool fault injection — media (6), posts (3), invites (3), sessions
    (1), password_resets (2), email_verifications (2), users
    `authenticate_with_closed_pool` (1). Reach handle via `state`, fault via
    `base.close_pool()`.
  - Magic-password / no-SQL internal-error tests — `users` corrupted-verify /
    hash-error paths that force errors without raw SQL.
  - Generic-store behavioral — `feed_cache` CRUD (4; `FeedCacheStore<DB>` is
    generic, `postgres/feed_cache.rs` exists), `posts` list-ordering;
    `sqlite/mod.rs` `create_user_with_invite_{hash_failure,insert_error}` (2,
    via `&*state.atomic`).
  - `test_storage_methods_with_lazy_pool_cover_error_paths`
    (`postgres/mod.rs:455`) — exercises error paths via `state`; convertible.
  - **Data-injection tests requiring the harness extension** — `users`
    `authenticate_with_corrupted_hash` (:17),
    `authenticate_with_invalid_email_in_db` (:53): the _behavior_ (bad row →
    `Internal`) is agnostic; they need a raw seed on each backend (see
    Component 1) + per-backend injection SQL.
- **Dedupe / delete (redundant):**
  - `make_postgres_app_state_constructs_with_lazy_pool` (:380),
    `test_storage_constructors` (:443) — pure PG constructor smoke tests already
    exercised by _every_ PG `setup()`; delete (coverage preserved by `setup()`),
    do **not** convert.
  - Any behavior already dual-covered in `server/tests/storage/storage.rs` (e.g.
    `set_password` at :1806; `test_session_record_from_row` largely duplicates
    the helpers.rs round-trip) — delete the dialect copy.
- **Decisively backend-specific → keep `sqlite_only`/`postgres_only` + reason
  (few):**
  - `postgres/bootstrap.rs`
    `create_postgres_database_and_role_attempts_admin_connection` —
    `CREATE ROLE`/`CREATE DATABASE`, no SQLite analog.
  - `sqlite/backup.rs` **5 async** tests (`export/restore rollback`,
    `validate_foreign_keys`, `schema_version`, `schema_checksum`) — SQLite
    `PRAGMA table_info`/`foreign_key_check`/ `sqlite_master` introspection; PG
    backup is a separate module. `sqlite_only`.
  - `users` `authenticate_with_blocked_update` (:74) — **audit decides:**
    convert with an equivalent PG update-blocking trigger, or keep `sqlite_only`
    if the SQLite `RAISE(FAIL)` mechanism has no non-contrived PG parallel.

### Generalizable pure-logic (dedup the helper, move the test)

- `quote_identifier` — identical in 4 places (`sqlite/backup.rs:307`,
  `postgres/backup.rs:306`, `postgres/bootstrap.rs:91`, `test_support.rs:319`; 8
  call sites in the last).
- `quote_literal`/`quote_postgres_literal` — identical in 2
  (`sqlite/backup.rs:311`, `postgres/bootstrap.rs:99`).
- `parse_status` — identical in 2 (`sqlite/feed_events.rs:12`,
  `postgres/feed_events.rs:114`; PG copy `cov:ignore`d as a dup);
  `FeedEventStatus` lives in root `feed_events.rs:13`.
- `user/invite_record_from_row` tests (`postgres/mod.rs`) — `pub(crate)` helpers
  in `helpers.rs`; move the **`Some`-branch** cases there (helpers.rs:750–808
  covers only the `None`/delegate path).

**Truly dialect-specific pure-logic (keep in place, guard-exempt):**
`insert_sql` (PG `CAST`/`OVERRIDING`), `json_select`, `bind_json_value`,
`pg_error_code_matches`, PG env-password/options resolution.

### Root non-dialect async (guard targets, not dialect work)

`smtp.rs` (7, mock store), `db.rs` (3, URL routing), `helpers.rs` (3, hashing),
`test_support.rs` `postgres_accessor_rejects_a_sqlite_pool` (:627, SQLite
type-guard) → `// guard:no-backend`. `test_support.rs`
`seed_user_creates_a_user` (:620) → convert to `#[apply(backends)]`
(backend-agnostic harness smoke test). Root `backup.rs` (3) → interim
`#[apply(sqlite_only)]` citing ADR-0019 + #136 (SQLite backup export/restore SQL
is dialect-specific; PG orchestration coverage is #136).

## Components (implementation order)

### 1. Verify-first audit + harness extension + file follow-ups

- **Audit** every one of the 49 dialect async tests (+ test_support's 2) into
  convert / dedupe-delete / decisive-keep, each with a written reason (the
  reason becomes the test's `// reason:` or its deletion note).
  Convert-by-default; a keep needs a decisive reason. **For each convert, record
  the target home module** (`storage/src/<trait>.rs`, derived from the
  `&*state.<handle>` it exercises — `media`→`media.rs`,
  `feed_cache`→`feed_cache.rs`, `atomic`→its home, etc.). Output: a per-test
  table (verdict, reason, source file, target home module) the later tasks
  consume.
- **Extend the harness** to make raw seeding backend-agnostic: add an agnostic
  `CloseablePool::execute(&self, sql: &str) -> Result<(), sqlx::Error>` that
  dispatches on the enum internally — the seed counterpart to `close()`. This is
  the clean shape (mirrors `close()`), **not** a typed `sqlite()` accessor
  mirroring `postgres()`'s asymmetry. (`postgres()` stays for pre-existing typed
  _inspect_ tests, which can't be made agnostic — out of scope.) In the spirit
  of #170.
- **File follow-up issues:** move `smtp.rs` out of storage (user-approved);
  unify the `insert_sql` placeholder token (`?n`→`$n`).

### 2. Dedup the generalizable pure-logic helpers + move tests

New `storage/src/sql.rs` (`mod sql;` in lib.rs) exposing
`pub(crate) fn quote_identifier`/`quote_literal`; repoint all four/two copies
incl. `test_support.rs` (`backup.rs:243 order_by_clause` already takes the
quoting fn as `fn(&str)->String` → pass `crate::sql::quote_identifier`); move
the identifier **and** literal tests to `sql.rs` as plain `#[test]`. Collapse
`parse_status` into root `feed_events.rs`, drop the PG `cov:ignore`, move its
all-arms test. Relocate the `record_from_row` `Some`-branch tests into
`helpers.rs`; delete `test_session_record_from_row` (redundant). `rg` finds one
definition of each deduped helper.

### 3. Convert **and re-home** the backend-agnostic dialect tests

Per the audit, **move** each convert target out of the dialect file into its
target home module `storage/src/<trait>.rs`'s `#[cfg(test)] mod tests` (create
the block + `use crate::test_support::{backends, Backend, TestEnv};` if absent,
or extend the existing one #126 added), and rewrite to the #242 template:
`#[apply(backends)] / #[tokio::test] / async fn …(#[case] backend: Backend)`,
`let TestEnv { state, base } = backend.setup().await;`, reach handles via
`&*state.<handle>`, fault via `base.close_pool()`, and — for the data-injection
tests — seed via the agnostic `base.pool().execute(sql)` with per-backend SQL.
Delete the redundant constructor/dup tests. As each dialect file's common tests
leave, **remove the now-dead scaffolding** (`sqlite/mod.rs:323 sqlite_pool()`
and per-module `pool()` helpers, e.g. `sqlite/feed_events.rs`,
`sqlite/feed_cache.rs`) and **delete the dialect file's `#[cfg(test)] mod tests`
entirely once only decisive-keep tests (if any) would remain** — else
`dead_code`/clippy fails. Net: no `#[apply(backends)]` test lives in a dialect
file; converted tests sit in the generic home module beside the `XStore<DB>`
they prove.

### 4. Annotate the decisive-keep residue + root tests

Decisive-keep tests **stay where their backend-specific code lives** (the
dialect file is their correct home — no generic module exists for them).
Annotate `#[apply(sqlite_only)]`/`#[apply(postgres_only)]` + a **decisive**
`// reason:` (`let _ = backend;` for the ignored param) on the bootstrap DDL
test, the 5 `sqlite/backup.rs` async tests, and (if kept) `blocked_update`.
`// guard:no-backend` on the 13 non-DB root tests + the `CloseablePool`
type-guard. Root `backup.rs` (3) → interim `sqlite_only` + ADR-0019/#136 reason.

### 5. Widen the guard to `storage/src` (xtask)

Generalize `test_pattern_check.rs::run()` to roots
`["server/tests", "storage/src"]`; keep the single step + "missing root is a
hard failure"; update the module-doc header. Add a `problems()` unit test over a
synthetic `storage/src/sqlite/foo.rs` bare `#[tokio::test]` (flagged) +
`#[apply(backends)]`/`guard:no-backend` (clean). **Last code task** — it goes
green only when all 114 `storage/src` tokio tests are annotated (the 47 root
`#[apply(backends)]` from #126 already pass; this issue resolves the other 67).

### 6. Record the convention (ADR)

Via `jaunder-adr` (numberless draft), recording the storage-test convention this
issue settles — and explicitly **superseding the stale "dialect files carry no
in-file tests (for coverage reasons)" belief** (the two-pass coverage rationale
no longer holds; coverage is a single workspace-wide PG-live pass, placement-
independent):

- **Home a test by what it proves, not which backend runs it** — backend-common
  (`#[apply(backends)]`) tests live in the generic home module
  `storage/src/<trait>.rs`; decisively backend-specific tests live with their
  dialect code.
- **Presume a coverage gap:** convert backend-agnostic storage tests to both
  backends via the fault/seed harness; keep single-backend only on a decisive
  backend-exclusive reason.
- Classify pure-logic by generalizability; dedup identical helpers, test once.
  Cross-reference ADR-0019, #126, and #170.

## Verification

- `cargo xtask check` (static + clippy + Nix coverage incl. Postgres) green
  after each task.
- **Coverage rises, never falls:** conversions keep the SQLite arm and add the
  Postgres arm; deleted constructor tests stay covered by `setup()`; deduped
  helpers covered once (dropping the PG `parse_status` `cov:ignore` is safe —
  the moved test hits all four arms); relocated `Some`-branch record tests
  preserve `build_*_record` branch coverage.
- Negative guard test: a bare dialect `#[tokio::test]` fails
  `test-backend-pattern`.
- `cargo xtask validate` green.

## Acceptance criteria

1. **Every convert-target dialect async test runs on both backends** via
   `#[apply(backends)]` + `backend.setup()` (+ `base.close_pool()` or
   `base.pool().{sqlite,postgres}()` seeding). Redundant/already-dual-covered
   copies are deleted, not duplicated. 1a. **Converted tests are re-homed to the
   generic home module** `storage/src/<trait>.rs` (beside the `XStore<DB>` they
   prove); **no `#[apply(backends)]` test remains in any
   `storage/src/{sqlite,postgres}/*.rs` dialect file**
   (`rg -c 'apply(backends)'` over the dialect dirs = 0). A dialect file's
   `#[cfg(test)]` block is deleted once no decisive-keep test remains in it.
2. **Every remaining single-backend async test carries a decisive
   backend-exclusive `// reason:`** — "error path"/"lazy pool"/"SQLite-written
   seed" is not decisive. Expected kept set: PG bootstrap DDL, the 5
   `sqlite/backup.rs` internals, the `CloseablePool` type-guard
   (`guard:no-backend`), and at most `blocked_update`.
3. **Agnostic `CloseablePool::execute(sql)` seed method added** (dispatches on
   the enum, mirroring `close()` — no typed `sqlite()` accessor); data-injection
   tests seed on both backends through it.
4. **Generalizable pure-logic helpers have one definition each**
   (`quote_identifier` ×4→1, `quote_literal` ×2→1, `parse_status` ×2→1; all call
   sites incl. `test_support.rs` compile; PG `parse_status` `cov:ignore`
   removed); their tests are plain `#[test]` in the shared modules.
5. **No `record_from_row` test remains in `postgres/mod.rs`;** `Some`-branch
   cases moved to `helpers.rs`.
6. **Truly dialect-specific pure-logic tests stay in place** (guard-exempt).
7. **Every `#[tokio::test]` under
   `storage/src/**`carries a backend template or`//
   guard:no-backend`** — machine-verified by the widened guard (incl. `test_support.rs`'s 2 and `sqlite/backup.rs`'s
   5); a missing root is a hard failure; a bare dialect test is unit-flagged.
8. **Dead scaffolding removed** (`sqlite_pool()`, per-module `pool()` helpers) —
   no `dead_code`/clippy warnings.
9. **An ADR draft** records the convention; **follow-up issues filed** (smtp.rs,
   `insert_sql` placeholder).
10. **`cargo xtask validate` green, coverage not lowered** (Postgres parity
    holes closed).

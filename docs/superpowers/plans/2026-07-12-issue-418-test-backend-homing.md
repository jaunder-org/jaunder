# Plan — #418: enforce ADR-0053 single-backend homing + correct the drift

**Spec:** `docs/superpowers/specs/2026-07-12-issue-418-test-backend-homing.md`
(the "what/why"; this plan is the "how"). **Issue:** #418 (+ #419 folded in).
**For agentic workers:** drive with `jaunder-iterate`, delegating a task to
`jaunder-dispatch` where useful. **Language:** complete Rust + exact `cargo`.

## Review header

**Goal.** Make the tree obey ADR-0053's guarantee (spec) — a `*_only` template ⟺
a dir named for its backend, and no test wears a backend template it discards —
by correcting the drifted single-backend tests, then strengthen
`test-backend-pattern` to enforce it (three composed rules + #419).
`cargo xtask validate` green at the end.

**Tasks (one line each):**

1. `cmd_create_pg_db_rejects…` → bare + `// guard:no-backend` (no DB).
2. Duals: `export_propagates_media_mirror_failure`,
   `pending_subscription_is_not_admitted`, `feed_events_marks_run` →
   `#[apply(backends)]`.
3. Migration trio → one bare `guard:low-level-db` test in
   `storage/src/postgres/migrations.rs`.
4. `pg_teardown` pair → `storage/src/postgres/teardown.rs` (one keeps
   `postgres_only` & uses `backend.setup()`; one bare `guard:low-level-db`).
5. Kind-③ relocations: `sqlite_pool_enforces_foreign_keys` →
   `storage/src/sqlite/pool.rs`, `every_foreign_key_is_deferrable` →
   `storage/src/postgres/schema.rs`, `claim_pending_batch_no_lock_contention` →
   `storage/src/sqlite/feed_events.rs` (fixing its discarded param).
6. `cmd_create_pg_db` provisioning pair → new
   `server/tests/misc/postgres/commands.rs` (bare `guard:low-level-db`);
   `backup_interop` trio → bare `guard:low-level-db` **in place** (cross-backend
   → generic home).
7. Strengthen `test-backend-pattern`: homing + param-honesty rules + the
   `guard:low-level-db` marker + unit tests.
8. #419: reason required on every `*_only` keep and every `guard:*` marker +
   unit tests.
9. Full `cargo xtask validate` green.

**Key facts (settled by audit + investigation — see spec):** every current
param-discarder uses the literal `let _ = backend;` tell (mechanically
detectable); the migration trio is the sole coverage of the public
`open_database` from-scratch path but is PG-only, redundant, and self-fixtured
(→ collapse to one bare low-level test); `claim_pending_batch` and both FK tests
touch only storage (→ move to `storage`); interop is genuinely cross-backend (→
stays generic, just marked).

## Disposition table

| test (current file)                                                                       | outcome                                                                       | home                                                    |
| ----------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------- |
| `export_propagates_media_mirror_failure` (`storage/src/backup.rs`)                        | → `#[apply(backends)]`, drop reason                                           | in place                                                |
| `pending_subscription_is_not_admitted` (`server/tests/storage/storage.rs`)                | → dual                                                                        | in place                                                |
| `feed_events_marks_run` (`storage.rs`)                                                    | → dual                                                                        | in place                                                |
| `cmd_create_pg_db_rejects_non_postgres_urls` (`server/tests/misc/commands.rs`)            | drop template/param → bare `// guard:no-backend`                              | in place                                                |
| 3× `open_*` migration (`storage.rs`)                                                      | **collapse to 1** bare `// guard:low-level-db`, honest reason, delete other 2 | `storage/src/postgres/migrations.rs` (new)              |
| `per_test_database_is_dropped_on_teardown` (`server/tests/misc/pg_teardown.rs`)           | keep `postgres_only`, `Backend::Postgres.setup()` → `backend.setup()`         | `storage/src/postgres/teardown.rs` (new)                |
| `unique_postgres_database_is_dropped_on_guard_drop` (`pg_teardown.rs`)                    | bare `// guard:low-level-db`                                                  | `storage/src/postgres/teardown.rs`                      |
| `sqlite_pool_enforces_foreign_keys` (`storage.rs`)                                        | keep `sqlite_only` (already uses param)                                       | `storage/src/sqlite/pool.rs` (new)                      |
| `every_foreign_key_is_deferrable` (`storage.rs`)                                          | keep `postgres_only` (already uses param)                                     | `storage/src/postgres/schema.rs` (new)                  |
| `claim_pending_batch_no_lock_contention` (`server/tests/feed/feed_events_concurrency.rs`) | keep `sqlite_only`, `Backend::Sqlite.setup()` → `backend.setup()`             | `storage/src/sqlite/feed_events.rs` (existing test mod) |
| 2× `cmd_create_pg_db` provisioning (`commands.rs`)                                        | bare `// guard:low-level-db`, split out                                       | `server/tests/misc/postgres/commands.rs` (new)          |
| 3× `backup_interop` (`server/tests/misc/backup_interop.rs`)                               | bare `// guard:low-level-db`, honest cross-backend reason                     | **in place**                                            |

## Global constraints

- **Ordering is load-bearing.** Tasks 1–6 each keep the _existing_ guard green
  (they preserve a template, or become a `guard:no-backend`/`guard:low-level-db`
  bare test — note: **widen the existing guard to accept `guard:low-level-db`
  first, in Task 1**, so the bare low-level tests in Tasks 3–6 don't trip it).
  The strengthened homing/param rules (Task 7) land only after the tree
  conforms.
- **Dual conversions must PASS on both backends**, not just compile.
- **Split, don't duplicate:** move each test's `use`s and file-local helpers
  with it (`db_name_from_url`/`database_exists` travel with `pg_teardown`);
  leave the source file compiling; delete a file (+ its `mod` line) if it
  empties.
- **In-crate storage tests are coverage-measured** — moving tests into
  `storage/src` shifts coverage attribution; production coverage must not drop
  (confirm at Task 9).
- Per `jaunder-commit`: `cargo xtask check` before each commit. **No
  `Co-Authored-By` trailer.** No edits during a gated commit
  (`project_no_edit_during_gated_commit`).
- New in-crate `#[cfg(test)] mod tests` reach the harness via
  `use crate::test_support::{…}` under `cfg(test)` — no feature work.
  `mod <file>;` goes in the dialect `mod.rs`
  (`storage/src/{sqlite,postgres}/mod.rs`); a new `server/tests/misc/postgres/`
  needs `mod postgres;` in `server/tests/misc/main.rs` + a `postgres/mod.rs`
  declaring its files.

---

## Task 0 (do first, inside Task 1's commit) — teach the existing guard `guard:low-level-db`

**Files:** `xtask/src/steps/test_pattern_check.rs`. **Change:** in
`is_exempt_or_tagged`, accept `// guard:low-level-db` alongside
`// guard:no-backend`, so a bare `#[tokio::test]` carrying it is not flagged by
the current template-or-marker rule. Add a unit test. This is what lets Tasks
3–6 introduce bare low-level tests without a red gate. **Run:**
`cargo nextest run -p xtask --manifest-path xtask/Cargo.toml test_pattern` →
PASS.

## Task 1 — the non-DB URL-validation test

**Files:** `server/tests/misc/commands.rs`. **Change:**
`cmd_create_pg_db_rejects_non_postgres_urls` returns before any DB. Remove
`#[apply(postgres_only)]` + `#[case] backend: Backend` + the false reason; keep
`#[tokio::test]`, add
`// guard:no-backend — pure URL validation, returns before any DB`. **Run:**
`cargo nextest run -p server --test misc cmd_create_pg_db_rejects` → PASS.
**Commit:** "test(misc): cmd_create_pg_db URL-validation test is not
backend-specific" (includes Task 0).

## Task 2 — three backend-common duals

**Files:** `storage/src/backup.rs`; `server/tests/storage/storage.rs`.
**Change:** replace each `*_only` with `#[apply(backends)]`, delete the
§2-non-decisive reason, confirm the body uses `backend.setup()`/`env.state` (it
does) and no dialect-specific SQL. **Run:**
`cargo nextest run -p storage export_propagates_media_mirror_failure`;
`cargo nextest run -p server --test storage pending_subscription_is_not_admitted feed_events_marks_run`
— all cases PASS on both backends. **Commit:** "test: convert three
backend-common tests to dual (ADR-0053 §2)".

## Task 3 — migration trio → one low-level test in storage

**Files:** new `storage/src/postgres/migrations.rs` (+ `mod migrations;` in
`postgres/mod.rs`); trim the three `open_*` tests from
`server/tests/storage/storage.rs`. **Change:** write ONE bare `#[tokio::test]` —
`open_database_migrates_a_from_scratch_database` — minting a fresh
`unique_postgres_url()` DB, calling public `open_database`, asserting a migrated
table (`site_config.get("missing") == None`). Marker:
`// guard:low-level-db — Postgres per-test DBs are template clones (setup bypasses migration); this is the sole test of the real migration run against a from-scratch DB via the public open_database. SQLite has no template, so every SQLite test covers its path.`
Imports via `crate::test_support::{unique_postgres_url}` +
`crate::open_database` (confirm path). Delete the other two + their misleading
names. **Run:**
`cargo nextest run -p storage open_database_migrates_a_from_scratch_database` →
PASS; `--test storage` still builds. **Commit:** "test(storage): one honest
from-scratch-migration test (ADR-0053 homing)".

## Task 4 — pg_teardown pair → storage

**Files:** new `storage/src/postgres/teardown.rs` (+ `mod teardown;`); delete
`server/tests/misc/pg_teardown.rs` + its `mod pg_teardown;` in `misc/main.rs`.
**Change:** move both tests + file-local `db_name_from_url`/`database_exists`
into a `#[cfg(test)] mod tests`; `use crate::helpers::{…}` →
`use crate::test_support::{…}`. `per_test_database…`: keep
`#[apply(postgres_only)]`, change `Backend::Postgres.setup()` →
`backend.setup()`, drop `let _ = backend;`, keep its reason.
`unique_postgres_database…`: bare `#[tokio::test]` +
`// guard:low-level-db — drives unique_postgres_url()/PostgresDbGuard directly, not the backend fixture`.
**Run:** `cargo nextest run -p storage teardown` → PASS; `--test misc` still
builds. **Commit:** "test(storage): home the test_support teardown tests in the
storage crate".

## Task 5 — kind-③ storage relocations (+ fix claim_pending_batch's param)

**Files:** new `storage/src/sqlite/pool.rs` (+ `mod pool;`) ←
`sqlite_pool_enforces_foreign_keys`; new `storage/src/postgres/schema.rs` (+
`mod schema;`) ← `every_foreign_key_is_deferrable`; existing
`storage/src/sqlite/feed_events.rs` test mod ←
`claim_pending_batch_no_lock_contention`; trim `server/tests/storage/storage.rs`
and `server/tests/feed/feed_events_concurrency.rs` (delete the latter + its
`mod` line if it empties). **Change:** move each into a
`#[cfg(test)] mod tests`; rewrite server-helper/`env.state` access to
`crate::test_support` + direct store/pool access (all three are
pool/catalog/`feed_events` calls). The two FK tests already use
`backend.setup()` — keep their `*_only` + reasons unchanged.
`claim_pending_batch`: keep `sqlite_only`, change `Backend::Sqlite.setup()` →
`backend.setup()`, drop `let _ = backend;`, keep `#[ignore]`

- the #18 reason. **Run:** `cargo nextest run -p storage foreign_key`;
  `cargo nextest run -p storage -- --ignored claim_pending_batch` → PASS; both
  server binaries still green. **Commit:** "test(storage): home the FK and
  lock-repro tests in dialect dirs".

## Task 6 — server low-level tests: cmd provisioning → misc/postgres/; interop marked in place

**Files:** new `server/tests/misc/postgres/mod.rs` (`mod commands;`) +
`server/tests/misc/postgres/commands.rs`; add `mod postgres;` to
`server/tests/misc/main.rs`; trim `server/tests/misc/commands.rs`; edit
`server/tests/misc/backup_interop.rs` in place. **Change:** move the two
`cmd_create_pg_db` provisioning tests to `misc/postgres/commands.rs` as bare
`#[tokio::test]` +
`// guard:low-level-db — provisions a Postgres role/database via bootstrap admin; no standard backend fixture`
(drop the templates + `let _ = backend;`). For the three `backup_interop` tests:
drop `#[apply(postgres_only)]` + `let _ = backend;`, make them bare
`#[tokio::test]` +
`// guard:low-level-db — cross-backend interop; drives both engines in one body and needs a live Postgres`
— left in place (generic home). **Run:**
`cargo nextest run -p server --test misc cmd_create_pg_db backup_interop` →
PASS. **Commit:** "test(misc): mark low-level DB tests; home PG-provisioning
under postgres/".

## Task 7 — strengthen `test-backend-pattern` (homing + param-honesty)

**Files:** `xtask/src/steps/test_pattern_check.rs`. **Interfaces:** add pure
functions folded into `problems()`:

- `homing_violations(path, source)` — `/sqlite/` flags
  `backends`/`backends_matrix`/`postgres_only`; `/postgres/` flags
  `backends`/`backends_matrix`/`sqlite_only`; generic flags
  `sqlite_only`/`postgres_only`.
- `param_honesty_violations(source)` — a
  `#[apply(backends|backends_matrix|sqlite_only|postgres_only)]` cluster whose
  body has `let _ = backend;` (or param `#[case] _backend`) → flag. Each emits
  `path:ln: <rule> — <recovery naming ADR-0053 §1/§2>`. **Test:** unit tests
  (mirror the existing table) for every branch: dual/`*_only` in each dir kind →
  clean/flagged as specified; generic `*_only` → flagged; `guard:low-level-db`
  bare test anywhere → clean; `let _ = backend;` under a template → flagged;
  `guard:no-backend`

* plain `#[test]` → clean. **Run:**
  `cargo nextest run -p xtask --manifest-path xtask/Cargo.toml test_pattern` →
  PASS; `devtool run -- cargo xtask check` → the guard passes on the
  now-conforming tree. **Commit:** "feat(xtask): test-backend-pattern enforces
  ADR-0053 homing + param honesty".

## Task 8 — #419: reason required on keeps and markers

**Files:** `xtask/src/steps/test_pattern_check.rs`; any survivor lacking an
honest reason. **Interfaces:** a `#[apply(sqlite_only|postgres_only)]` cluster
must contain a `// reason:`; a `// guard:no-backend`/`// guard:low-level-db`
marker must have a non-empty trailing reason. **Test:** `*_only` without
`// reason:` → flagged; bare `guard:*` marker → flagged; justified forms →
clean. **Run:** `cargo nextest run -p xtask --manifest-path xtask/Cargo.toml` →
PASS; `devtool run -- cargo xtask check` green. **Commit:** "feat(xtask):
require a decisive reason on single-backend keeps + markers (#419)".

## Task 9 — full gate

**Run:** `devtool run -- cargo xtask validate` (background; cold Nix rebuild) —
static + clippy + coverage + the full `{sqlite,postgres}×{chromium,firefox}` e2e
matrix, guard green. Confirm coverage did not regress from moving tests into
`storage/src`. No commit unless fallout forces edits.

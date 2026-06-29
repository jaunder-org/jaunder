# Spec — Issue #127: dual-backend conversion + suite-wide guard across `server/tests`

**Issue:** [#127](https://github.com/jaunder-org/jaunder/issues/127) — *test-infra: extend
dual-backend conversion + uniform-pattern guard across remaining server/tests*
**Milestone:** 5 — Backend-parity test coverage
**Builds on:** #54 (the `test-backend-pattern` guard + the `#[apply(backends)]` convention)
**Date:** 2026-06-29

## Problem

#54 made `server/tests/storage/storage.rs` fully backend-explicit and added the
`test-backend-pattern` xtask guard, but pointed the guard at that one file only. The rest of
`server/tests/**` still contains **56 bare `#[tokio::test]`s across 16 files** that either run
SQLite-only or are backend-implicit (one env-chosen backend per run, not rstest-expanded into
both cases). The guard does not yet police them, so backsliding is unguarded suite-wide.

## Goal

Make every DB-touching server integration test backend-explicit (`#[apply(backends)]` for
dual-backend; `#[apply(sqlite_only)]`/`#[apply(postgres_only)]` for deliberate single-backend,
each with a reason), remove genuinely non-DB tests from the integration suite or exempt them
explicitly, and **widen the guard to police all of `server/tests`** so the invariant holds
suite-wide.

## Current state (audited)

Bare-test census (template-less `#[tokio::test]`), by classification:

| Bucket | Count | Files |
|---|---|---|
| **(a) Already-dual, wrong syntax** — use `#[values(Backend::Sqlite, Backend::Postgres)]` + `backend.setup()`; run both backends already, just not via the template | 20 | web_posts ×4, atompub_posts ×4, web_media ×2, web_backup ×1, web_auth ×1, atompub_media ×1, atompub_rsd ×1, feed_events_hook ×1, feed_handlers ×1, media_handlers ×1 |
| **(b) Backend-implicit / incidentally-SQLite — convert to `#[apply(backends)]`** | 25 | `misc/commands.rs` ×24 (env-branching `storage_args()` helper; one backend per run); `feed_worker::worker_applies_backoff_on_ping_failure` ×1 (hand-builds a `Sqlite*`-typed `AppState` from a raw `SqlitePool` — incidental SQLite; backoff-on-ping-failure is backend-agnostic) |
| **(c) Genuinely single-backend (verified by reading the test for an intentional reason)** | 6 | commands.rs PG-provisioning ×3 (`postgres_only` — need PG admin/bootstrap); backup_interop ×2 (`postgres_only` — cross-backend backup needs live PG); pg_teardown ×1 (`postgres_only` — PG per-test-DB teardown); feed_events_concurrency ×1 (`sqlite_only` — issue #18 SQLite lock-flake reproduction: reserved-lock upgrade under `busy_timeout`, which Postgres MVCC structurally can't exhibit; also `#[tokio::test(flavor = "multi_thread")]`, `#[ignore]`d) |
| **(d) Genuinely non-DB** | 3 | static_assets ×2; web_auth pure-auth-logic ×1 |

(c) sums to 7 once feed_events_concurrency is counted with the three `postgres_only` commands and
the two backup_interop + pg_teardown. Total ≈ 20+25+7+3 = 55, ±2 for ambiguous singles in
atompub_rsd — the per-file audit in the plan resolves the exact set.

**Classification rule (binding on the plan): "currently hardcodes SQLite" is NOT a reason for
`sqlite_only`.** Every test placed in bucket (c) must be justified by reading it and finding an
*intentional* single-backend reason (asserts backend-specific behavior the other backend can't
exhibit), recorded in its `// reason:` comment. Incidental single-backend construction (a
hand-built `Sqlite*` `AppState`, a raw `SqlitePool`, an env-branch) is bucket (a)/(b) — convert it.
The `feed_worker` reclassification above is the worked example of this rule.

Key enablers confirmed: the CLI `StorageArgs.db` is already `DbConnectOptions` (both backends);
`open_database`/`open_existing_database` dispatch on it; backup/restore is backend-agnostic NDJSON;
`unique_postgres_url()` yields a fresh unmigrated PG database (what `init` tests need). **No source
changes are required** — every conversion lives in the test layer.

## Design

### Part A — Convert the 20 `#[values]` tests → `#[apply(backends)]`

Each already takes `#[case] backend: Backend` (or an rstest `#[values(...)]` arg) and calls
`backend.setup().await`. Replace the inline `#[values(Backend::Sqlite, Backend::Postgres)]` with the
shared `#[apply(backends)]` template; bodies and assertions are unchanged. This is a syntax
standardization (so the guard's single rule holds) — **no coverage change**.

Plus one conversion of a different shape: `feed_worker::worker_applies_backoff_on_ping_failure`
hand-builds a `Sqlite*`-typed `AppState` from a raw `SqlitePool` + a manual `sqlx::migrate!`.
Replace that whole preamble with `#[apply(backends)] … (#[case] backend: Backend)` +
`let env = backend.setup().await; let state = &env.state;`, keeping the `FailingWebSubClient` and
the backoff assertions unchanged — this one **does** close a real Postgres coverage hole.

### Part B — Convert `misc/commands.rs` to backend-explicit (24 dual + 3 PG-only)

Replace the env-branching helper `storage_args(base: &TempDir)` with a backend-parameterized
`storage_args(backend: Backend, base: &TempDir)` that selects the URL by backend
(`Backend::Sqlite => sqlite_url(base)`, `Backend::Postgres => unique_postgres_url().await`) instead
of `if postgres_testing_enabled()`. Do the same for `uninitialized_storage_args`
(`nonexistent_postgres_url()` on the Postgres arm). Then add `#[apply(backends)] … (#[case] backend:
Backend)` to each of the 24 DB tests, threading `backend` into the helper. The CLI command calls
(`cmd_init`, `cmd_user_create`, `cmd_app_password_create`, `cmd_backup`, `cmd_restore`, …) and all
assertions are unchanged — they already operate through `DbConnectOptions`. The 3 PG-provisioning
tests (`cmd_create_pg_db_*`) get `#[apply(postgres_only)]` + `let _ = backend;` + a reason comment
(they need PG admin/bootstrap; the existing pattern from #54). **No source changes.**

### Part C — Annotate the genuinely single-backend tests (bucket c)

`#[apply(postgres_only)]` for the 3 `cmd_create_pg_db_*` provisioning tests (need PG
admin/bootstrap), backup_interop ×2 (cross-backend backup needs live PG), and pg_teardown ×1 (PG
per-test-DB teardown). `#[apply(sqlite_only)]` for feed_events_concurrency ×1 — **reason: reproduces
the SQLite-specific issue #18 `claim_pending_batch` lock flake (reserved-lock upgrade under
`busy_timeout`); Postgres MVCC can't exhibit it**. Each carries a `// reason: …` comment. The
`feed_events_concurrency` test keeps its `#[tokio::test(flavor = "multi_thread", worker_threads =
4)]` form (and its `#[ignore]`) — which forces the guard change in Part E.

Note: `feed_worker::worker_applies_backoff_on_ping_failure` is **NOT** here — it is bucket (b)
(incidental SQLite), converted to `#[apply(backends)]` in Part B by replacing its hand-built
`Sqlite*` `AppState` with `backend.setup().await` → `env.state`.

### Part D — Non-DB tests: relocate unit-shaped ones, exempt genuine ones

Per the policy: a test that is really a unit test belongs in unit-test space; a genuine non-DB
*integration* test stays but is explicitly exempted. The plan classifies each of the 3 individually
by reading it:
- If it exercises only a pure function / extractor with no router or app wiring (likely
  `web_auth::auth_user_extraction_fails_without_session_storage_extension`), **relocate** it to a
  `#[cfg(test)] mod tests` unit test in the crate that owns the code, removing it from
  `server/tests` entirely (so the guard never sees it).
- If it genuinely exercises integration wiring but touches no DB (possibly the `static_assets`
  asset-serving tests, if they go through the axum asset route), **keep it and add an exemption
  marker** the guard recognizes (Part E): a `// guard:no-backend — <reason>` comment in its
  attribute block.

### Part E — Guard: widen suite-wide + harden (`xtask/src/steps/test_pattern_check.rs`)

1. **Scope → directory walk.** Replace `const SCANNED: &[&str]` (the single storage.rs path) with a
   recursive walk of `server/tests/**/*.rs`. A missing root directory is a **hard failure**, not a
   silent pass (closes #54 follow-up 2).
2. **Match parameterized forms.** Treat a line as a tokio test when its trimmed text is
   `#[tokio::test]` **or starts with `#[tokio::test(`** (closes #54 follow-up 1; required by
   `feed_events_concurrency`).
3. **Exemption marker.** A bare `#[tokio::test]` whose attribute block contains a
   `// guard:no-backend` comment is treated as a declared non-DB test and skipped. The marker must
   carry a trailing reason (`// guard:no-backend — <reason>`); the guard requires the prefix.
4. **Contiguity tolerance.** When scanning a test's attribute block for the template/marker, the walk
   skips interspersed blank lines and `//`/`///` comment lines (stopping only at a non-attribute code
   line or the `fn`), so a doc-comment between `#[apply(...)]` and `#[tokio::test]` no longer
   false-positives (closes #54 follow-up 3).
5. Unit tests (sync `#[test]`) remain exempt; the existing fixture unit tests are extended to cover
   the parameterized form, the exemption marker, and the contiguity case.

The guard's `problems()`/`violations()` stay pure and fixture-tested; only `run()` changes from
reading one file to walking the directory.

## End state / acceptance

- Every DB-touching test under `server/tests/**` carries exactly one of the three templates;
  deliberate single-backend tests carry a `// reason:`; genuine non-DB integration tests carry
  `// guard:no-backend — <reason>`; unit-shaped tests have been moved out of the suite.
- The `test-backend-pattern` guard scans **all of `server/tests`** and fails on any unannotated,
  un-exempted bare/parameterized tokio test; it fails (not no-ops) if the test root is missing.
- `cargo xtask validate` is green (the Postgres cases run in the coverage pass).

## Testing

- The converted tests are the delta; they must pass on both backends (the coverage pass sets
  `JAUNDER_PG_TEST_URL`).
- The guard ships extended fixture unit tests (parameterized form fails-when-bare / passes-when-
  tagged; exemption marker; contiguity with an interleaved doc-comment).
- Full local gate: `cargo xtask validate`. (Gate runner is now `devtool run -- cargo xtask …` per
  the updated CLAUDE.md / #158; pre-commit hook still gates each commit.)

## ADR

No new architectural decision — extends existing ADR-0019/0021 patterns, the #54 guard, and the
db-test-harness. The `// guard:no-backend` exemption marker is a mechanical guard convention, not an
architecture choice; it will be documented in `CONTRIBUTING.md`'s testing section rather than an ADR.

## Out of scope (sibling issues)

- Storage-crate dialect-file test reconciliation + storage-crate guard variant → **#135** (reuses
  this guard's directory-walk seam).
- Backup/restore orchestration on Postgres → **#136**.

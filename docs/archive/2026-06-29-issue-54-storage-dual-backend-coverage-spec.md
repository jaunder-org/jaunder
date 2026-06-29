# Spec — Issue #54: dual-backend storage test coverage + uniform-pattern guard

**Issue:** [#54](https://github.com/jaunder-org/jaunder/issues/54) — *test-infra: storage tests for
divergent-impl functions are SQLite-only, leaving Postgres coverage holes*
**Milestone:** 5 — Backend-parity test coverage
**Date:** 2026-06-29

## Problem

`server/tests/storage/storage.rs` runs most tests on both backends via `#[apply(backends)]`
(`storage/src/test_support.rs:173`), which expands each into a `::sqlite` and a `::postgres`
case (the Postgres case runs when `JAUNDER_PG_TEST_URL` is set, as the xtask coverage/e2e passes
do). But **42** plain `#[tokio::test]`s instantiate SQLite types directly (`SqliteAtomicOps`,
`SqliteUserStorage`, `open_pool`, …) and therefore run on **SQLite only**.

The dangerous subset covers functions whose SQLite and Postgres implementations **diverge** —
the hand-written `AtomicOps` methods and the explicit-transaction dialect storage methods —
exactly where the two backends can disagree, yet only the SQLite branch is exercised. The
headline instance: `AtomicOps::create_user_with_invite` has fully separate SQLite
(`storage/src/sqlite/mod.rs:149`) and Postgres (`storage/src/postgres/mod.rs:76`) impls, but its
`InviteExpired` / `InviteNotFound` / `UsernameTaken` error paths and the rollback assertions
live only in SQLite-only tests. These tests were never converted *because* the impls diverge
(SQLite-typed bodies don't map onto the shared `TestEnv`/`AppState` handles), not because they
assert SQLite-specific behavior.

## Goal

Make every backend-agnostic storage test assert on **both** backends, make every genuinely
single-backend test a deliberate annotated choice, and add a guard so a new SQLite-only storage
test cannot slip in silently.

## Current state (audited)

- Templates `backends`, `sqlite_only`, `postgres_only` **already exist** and are exported
  (`storage/src/test_support.rs:156-178`). No new template work.
- Counts in `storage.rs`: 114 `#[apply(backends)]`, 2 `#[apply(sqlite_only)]`,
  2 `#[apply(postgres_only)]`, **42 plain `#[tokio::test]` (SQLite-only)**, 2 pure-sync `#[test]`
  unit tests (no backend).
- All divergent functions are reachable through the shared `Arc<AppState>` handle:
  `state.atomic.create_user_with_invite` / `…confirm_password_reset`,
  `state.email_verifications.use_email_verification`, `state.password_resets.*`,
  `state.users.*`, `state.sessions.*`, `state.invites.*`, `state.site_config.*`. A
  `#[apply(backends)]` test obtains the handle via `let env = backend.setup().await;
  let state = &env.state;` (see `invite_and_atomic_registration_work`, `storage.rs:721`).
- `confirm_password_reset` **already** has dual-backend tests
  (`confirm_password_reset_hash_failure_returns_internal(#[case] backend)`,
  `confirm_password_reset_bogus_token_returns_not_found_without_hashing`). The audit confirms its
  happy/rollback paths and converts any SQLite-only stragglers — no net-new method coverage needed.
- `build_mailer_returns_noop_when_smtp_not_configured` builds a mailer from `site_config` (present
  on both backends) — **not** backend-divergent, so it converts to `#[apply(backends)]` like the
  rest. There is **no** genuinely backendless `#[tokio::test]` to special-case.
- **No guard exists yet.** Issues #135 and #127 both reference "the guard introduced in #54".

## Design

### Part A — Convert the backend-agnostic tests (~36)

Each plain `#[tokio::test]` whose behavior is backend-agnostic is rewritten to the existing
dual-backend pattern:

```rust
#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    // … route through state.atomic / state.invites / state.users instead of Sqlite* types
}
```

The direct `SqliteAtomicOps::new(pool)` / `Sqlite*Storage::new(pool)` / `open_pool` instantiations
are deleted in favor of the shared handles. Covers: `create_user_with_invite` error/rollback paths
(the headline hole), `use_email_verification`, password-reset create/use, invite lifecycle,
user/session detail, SiteConfig get/set, and `build_mailer`.

### Part B — Annotate the genuinely single-backend tests (the standard mechanism)

The handful that are intrinsically one-backend (the Postgres init/migration tests such as
`open_database_runs_postgres_migrations_on_existing_empty_db`,
`open_database_succeeds_on_postgres_test_vm`,
`open_existing_database_runs_postgres_migrations_on_unmigrated_db`; any SQLite-specific case)
convert to `#[apply(postgres_only)]` / `#[apply(sqlite_only)]` — **the same rstest mechanism** —
routed through `backend.setup()` / `env.base` where feasible, each carrying a `// reason: …`
comment stating why it is single-backend. No bare `#[tokio::test]`-on-a-hardcoded-pool survives.
(The two pure-sync `#[test]` parse-time unit tests are not backend tests and are left as-is.)

### Part C — Uniform-pattern guard (new xtask check)

A new `cargo xtask` check (sibling to the coverage/crap checks) scans a **configured set of test
paths** — for #54, just `server/tests/storage/storage.rs` — and **fails** if any `#[tokio::test]`
is not immediately preceded by an `#[apply(backends)]` / `#[apply(sqlite_only)]` /
`#[apply(postgres_only)]`. Pure sync `#[test]` unit tests are exempt (no backend). On failure it
prints the offending `file:line` plus the one-line fix. The scanned-path set is the seam #127
widens suite-wide and #135 reuses for the storage crate.

- Runs in the static / `check` pass (no test execution needed — it is source analysis).
- Has its own xtask unit test: a fixture with a bare `#[tokio::test]` makes the check fail; the
  annotated form passes.

### Orphaned-helper removal

Conversion orphans these SQLite-typed helpers (currently used only by the converted tests); all
are deleted in the same change. Because tests compile under `-D dead_code`, an orphan is a hard
build failure, so `cargo xtask check` guarantees none is left behind:

`open_pool`, `user_storage`, `storage_pair`, `email_verification_storage`, `invite_storage_triple`,
`password_reset_storage`, and `open_pg_pool` (if the Postgres-init tests route through
`backend.setup()`). Retained: `sqlite_url` (lives in `test_support`, used by `Backend::setup`),
`username` / `password` / `make_*` / `raw_exec` / `lookup_names` (used by retained dual-backend
tests).

## End state / acceptance

- Divergent-impl storage functions (starting with `create_user_with_invite`, and confirming
  `confirm_password_reset`) have their error/rollback paths asserted on **both** backends.
- Every `#[tokio::test]` in `storage.rs` carries exactly one of the three templates; any remaining
  single-backend test is intentional and annotated with a stated reason.
- The new guard fails CI if a new bare `#[tokio::test]` appears in the scanned set.
- `cargo xtask validate` is green (Postgres cases actually run, since the coverage pass sets
  `JAUNDER_PG_TEST_URL`).

## Testing

- The converted tests are the test delta; they must pass on both backends.
- The guard ships with an xtask unit test (fixture fail/pass).
- Full local gate: `cargo xtask validate`.

## ADR

No new architectural decision — this applies existing **ADR-0019** (generic storage backend via
dialect) and **ADR-0021** (SQLite transaction discipline) and the existing rstest templates. The
guard is a mechanical policy, not an architecture choice, so no ADR is added. (If discoverability
of the guard policy later proves valuable, a short ADR can be added then.)

## Out of scope (sibling issues)

- Other `server/tests` files + widening the guard suite-wide → **#127**.
- Storage-crate dialect-file test reconciliation + storage-crate guard variant → **#135**.
- Backup/restore orchestration on Postgres → **#136**.

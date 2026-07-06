# Spec — issue #136: reconceive backup testing at the contract level

- Issue: [#136](https://github.com/jaunder-org/jaunder/issues/136)
- Milestone: Backend-parity test coverage
- Date: 2026-07-05
- Depends on / relates to: ADR-0053 (backup carve-out), ADR-0019 (backup
  excluded from dialect dedup), ADR-0050 (stateless coverage gate), issue #4
  (post_audiences fidelity gap).

## Problem

Backup/restore is a cross-backend **contract** — a backup is a _portable dump_
(ADR-0019, ADR-0053). #135 left 8 interim `#[apply(sqlite_only)]` backup tests
pointing here as placeholders, none truly backend-exclusive:

- `storage/src/backup.rs` (3) — orchestration tests that bypass `AppState`
  (hardcoded `sqlite://` URLs, raw pools, on-disk ndjson/manifest assertions):
  `restore_backup_rejects_missing_db_directory`,
  `export_backup_writes_ndjson_media_and_manifest`,
  `archive_backup_round_trips_database_and_media`.
- `storage/src/sqlite/backup.rs` (5) — tests of _implementation internals_ (the
  wrong altitude), sharing a throwaway `migrated_pool()`/`migrated_conn()`
  helper: `export_database_triggers_rollback_on_write_failure`,
  `restore_database_triggers_rollback_on_import_failure`,
  `validate_foreign_keys_reports_violations`,
  `schema_version_returns_migration_count` (brittle `== 22`),
  `schema_checksum_returns_nonempty_hex_string`.

## Key finding that shapes this spec

The per-backend round-trip is **not missing** — it already runs dual-backend at
the contract (CLI-command) level:

- `server/tests/misc/commands.rs::cmd_restore_restores_directory_backup`
  (`#[apply(backends)]`) is a full directory round-trip on both backends, using
  `populate_backup_fixture` / `assert_backup_fixture_restored`.
- `commands.rs` also covers, dual-backend, every CLI-wrapper branch and restore
  precondition: `cmd_backup_writes_directory_backup`,
  `cmd_backup_without_path_writes_under_storage_backups`,
  `cmd_restore_refuses_missing_backup_path`,
  `cmd_restore_refuses_populated_database`,
  `cmd_restore_refuses_nonempty_media_directory`,
  `cmd_restore_empty_target_rejects_invalid_backup`.
- `server/tests/misc/backup_interop.rs` covers both cross-backend hops
  (`sqlite_backup_restores_into_postgres`,
  `postgres_backup_restores_into_sqlite`).

`cmd_backup`/`cmd_restore` (`server/src/commands.rs`) are 3-line wrappers over
`storage::export_backup`/`restore_backup` (default-path + a not-exists check +
`ensure_restore_target_empty`), so the CLI-level tests already exercise the
storage orchestration on both backends. Rebuilding round-trips in the storage
crate would **duplicate** this at a lower altitude.

## Decisions

Two decisions below (DEC-A, DEC-B) were made in the user's absence after the
design interview and **await ratification at spec approval**; the interview
settled the other points.

- **DEC-A — home gap-filling at the contract/CLI level, do not relocate to the
  storage crate.** Rationale: the dual-backend round-trip already lives at the
  CLI level; a storage-crate rebuild over `export_backup`/`restore_backup`
  duplicates it. This revises the interview's tentative "test the storage layer
  directly" answer, whose premise ("the CLI is thin marshalling") is _confirmed_
  but outweighed by the fact that the thin-CLI test already exists dual-backend.
  Placement is coverage-neutral (ADR-0053: single workspace-wide instrumented
  pass with Postgres live), so nothing is lost by keeping tests at the
  CLI/server level.
- **DEC-B — keep both single-hop cross-backend tests and add the cycle.** Per
  ADR-0053 ("keep/formalize the cross-backend hops, both directions") and
  because focused single-hop tests localize which direction broke; the A→B→A→B
  cycle is an additional completeness proof, not a replacement.
- **Error paths are reconceived as dual-backend negatives** (interview), through
  the public interface — not kept `sqlite_only`. "Error path" is not a decisive
  single-backend reason (ADR-0053 §2).
- **A→B→A→B in one comprehensive test** (interview): a single cross-backend
  cycle proving fidelity by (a) asserting fixture **values** survive every hop
  and (b) byte-equality of the same-backend Postgres dump pair, rather than two
  separate A→B→A tests.
- **DEC-C — uniform restore-failure contract (product change)** (interview,
  follow-up): normalize backup-restore failure across backends — map Postgres
  FK/constraint failures to `BackupError::ConstraintViolation`, and move
  SQLite's `foreign_key_check` to _before_ `COMMIT` so a violation **rolls
  back** (instead of committing invalid data, then erroring). Both backends then
  fail a constraint-violating restore identically: `ConstraintViolation` +
  target unmodified. This is a small product change in a test-infra issue,
  chosen deliberately because a uniform contract is the whole point of #136; it
  also fixes a latent SQLite integrity wart (a rejected restore currently leaves
  the bad data committed).
- **DEC-D — interop is value-level; no canonicalization** (interview,
  follow-up): cross-backend restore already preserves timestamp _values_ (the
  existing interop test decodes a Postgres-written timestamp back through
  sqlx-sqlite and passes). #136 **asserts** that value fidelity (to Postgres's
  microsecond resolution) rather than assuming it, but does **not** add
  canonical timestamp serialization. Cross-backend dump _bytes_ may differ
  (cosmetic); byte-equality is asserted only for the same-backend Postgres pair.
  Canonicalization (rendering both backends to one ISO form on export, as
  booleans already are) is a deliberate non-goal here.
- **The post_audiences gap is out of scope** (interview): expand the existing
  issue #4, adding both the missing visibility tables **and** an anti-regression
  framework so a future table can't be silently dropped from backups.

## Scope

### Delete (redundant or wrong-altitude)

- `storage/src/backup.rs`: the 3 `sqlite_only` orchestration tests. Keep every
  pure `#[test]` helper test (order*by_clause, ensure*\*, mirror/restore media,
  read_table_rows, json_value_as_restore_text, manifest validation, …). Drop
  now-unused test imports (`sqlite_only`, `Backend`, `rstest`, `rstest_reuse`,
  `FromStr` if unused).
- `storage/src/sqlite/backup.rs`: the 5 `sqlite_only` tests **and** the
  `migrated_pool`/`migrated_conn` helpers. Keep the 3 pure `#[test]` tests
  (`json_select_marks_boolean_values_as_json_booleans`,
  `insert_sql_uses_numbered_placeholders`,
  `bind_json_value_accepts_all_json_shapes`). Drop now-unused test imports.

Their happy-path coverage is subsumed by `commands.rs`'s dual-backend round-trip
and by transitive coverage of `schema_version`/`schema_checksum`/
`validate_foreign_keys` from any round-trip. The only genuine coverage losses
are addressed by the additions below.

### Add — contract-level, at the CLI/server-test layer

Every negative test constructs a **valid** backup (seed via
`populate_backup_fixture` → `cmd_backup`), then tampers the on-disk
`db/*.ndjson` before `cmd_restore` into a fresh target — driving the failure
through the public interface, never a dialect internal.

1. **Archive round-trip, dual-backend** (`commands.rs`, `#[apply(backends)]`):
   mirror `cmd_restore_restores_directory_backup` with `BackupMode::Archive`.
   Closes the one real coverage gap from deletion (archive export + archive
   extraction on restore); `commands.rs` currently tests Directory mode only.
2. **Missing-`db/`-directory negative, dual-backend** (`#[apply(backends)]`):
   remove the `db/` directory from a valid backup, restore, assert
   `BackupError::InvalidBackup`. This is the **only** replacement for the
   deleted `restore_backup_rejects_missing_db_directory`; that error branch
   (`storage/src/backup.rs:204-209`, in the generic `restore_directory_backup`)
   is reached by **no** other test —
   `cmd_restore_empty_target_rejects_invalid_backup` fails earlier at
   `read_manifest` ("missing manifest") and never reaches the db-dir check.
   (Review finding — otherwise a CRAP regression.)
3. **Malformed-row rollback negative, dual-backend** (`#[apply(backends)]`):
   corrupt a row in a **non-first** exported table — `posts` (export index 6),
   not `site_config` (index 0) — so that earlier tables (`users`, index 1) are
   inserted inside the transaction before `read_table_rows` rejects the bad row.
   Assert the restore fails with `InvalidBackup` **and** the earlier table's
   rows are absent from the target (a genuine pre-commit rollback proof, not
   vacuous). Both backends wrap the import in a transaction and roll back here.
4. **Dangling-FK negative, dual-backend** (`#[apply(backends)]`): tamper an
   exported table so a foreign key dangles, restore, and assert — after the
   **uniform restore-failure product change** below — that the restore fails
   with `BackupError::ConstraintViolation` **and** leaves the target unmodified,
   on **both** backends. (Before the product change the two engines diverged in
   both error variant and post-failure state; the change makes them uniform —
   see "Product change" below.)
5. **A→B→A→B full-cycle fidelity** (`backup_interop.rs`,
   `#[apply(postgres_only)]`, gated by `postgres_testing_enabled()`): seed
   SQLite (A) → backup/restore into Postgres (B) → into SQLite (A₂) → into
   Postgres (B₂). Prove fidelity two ways:
   - **Functional, every hop:** `assert_backup_fixture_restored` at B, A₂, and
     B₂ (the fixture survives all four hops).
   - **Byte-level, soundly:** compare the two **Postgres-side** exported dumps —
     `E_B₁ == E_B₂` — over `db/*.ndjson` **and** `manifest.json` with only its
     `timestamp` field excluded. This is the **unconditional floor**: both are
     the same backend's deterministic export of the same logical data. Excluding
     only the timestamp (not the whole manifest) means `version`,
     `schema_version`, `schema_checksum`, `mode`, and `tables` **are** validated
     — for the same-backend pair `schema_checksum` is identical (it is only
     backend-divergent _across_ backends, which never enters this same-backend
     comparison).

   Exclude **only** the `timestamp` field — parse each `manifest.json`, drop
   that one key, compare the rest. It is a fresh `Utc::now()` on every export,
   differing purely because time moved on; that single field is the whole of the
   "manifest differs" effect.

   For the SQLite pair `E_A₁ == E_A₂` (`E_A₁` = original app-written seed;
   `E_A₂` = SQLite export after a Postgres round-trip): the stored values don't
   change with wall-clock, so this holds **iff** SQLite and Postgres render a
   given stored timestamp to byte-identical text. Whether they do is
   **unverified** — do not assume it either way. The plan **verifies it
   empirically** and asserts `E_A₁ == E_A₂` (db/\*.ndjson + manifest sans
   timestamp) only if it actually holds; if it doesn't, that is a real
   (cosmetic) cross-backend serialization nuance to note, and the A-leg falls
   back to the functional value assertion above. `E_B₁ == E_B₂` stands
   regardless.

### Consolidate + strengthen the fixture (value interop)

`populate_backup_fixture` / `assert_backup_fixture_restored` are currently
**duplicated** in `commands.rs` and `backup_interop.rs`. Move them to one shared
location used by both, so the fixture has a single definition.

While consolidating, **strengthen** the fixture to prove value interop (DEC-D):
`populate_backup_fixture` seeds a **fixed, microsecond-precision**
`published_at` (not `Utc::now()` — so the expected value is deterministic and
safe from Postgres's µs quantization), and `assert_backup_fixture_restored`
asserts the restored `published_at` **equals** that value. Every test using the
fixture (per-backend round-trips, cross-backend hops, the cycle) then proves
timestamps survive with their value, closing the current gap where only
non-timestamp fields are checked.

### Product change — uniform restore-failure contract (DEC-C)

Small, contained product edits so a constraint-violating restore fails
identically on both backends:

- `storage/src/postgres/backup.rs` (restore path): map an insert-time
  integrity-constraint failure (SQLSTATE class `23`, e.g. `23503`
  foreign-key-violation) to `BackupError::ConstraintViolation` instead of
  letting it propagate as `BackupError::Sqlx`.
- `storage/src/sqlite/backup.rs` (`restore_database`): run
  `validate_foreign_keys` (`PRAGMA foreign_key_check`, which works with
  `foreign_keys = OFF`) **inside** the transaction _before_ `COMMIT`; on
  violation `ROLLBACK` and return `ConstraintViolation`. Today it commits first
  and checks after, leaving invalid data committed on a rejected restore — the
  wart this fixes.

Net contract: `restore_backup` on either backend rejects a constraint-violating
backup with `ConstraintViolation` and leaves the target unmodified. This is the
minimal product footprint; no other backup/restore behavior changes.

### Coverage bookkeeping (precise per-arm)

Deleting the 8 tests moves coverage on the dialect error arms; the additions
above must leave the CRAP gate green. Exact state (verified against the code):

- **Export `Err`/rollback arm** — impractical to trigger through public
  `export_backup` (it always creates the `db/` subdir before the dialect
  writes), so the deleted `export_database_triggers_rollback_on_write_failure`
  called the dialect fn directly.
  - `postgres::backup::export_database` — **already** `// cov:ignore`d
    (`postgres/backup.rs:56-60`); leave as is.
  - `sqlite::backup::export_database` — currently live-covered by the deleted
    test; add a **new** `// cov:ignore` (`sqlite/backup.rs:55-58`) with a
    reason.
- **Restore `Err`/rollback arm** — now **re-covered** by the new negatives on
  both backends (malformed-row on both; dangling-FK on both — after the DEC-C
  SQLite change, SQLite also rolls back on a FK violation, so its negative
  reaches the arm).
  - `sqlite::backup::restore_database` (not currently ignored) — covered by the
    malformed-row and dangling-FK negatives; no marker.
  - `postgres::backup::restore_database` — **currently** `// cov:ignore`d
    (`postgres/backup.rs:89-93`); the malformed-row / dangling-FK negatives now
    reach it, so **remove** that marker (coverage rises, never falls).
- **SQLite `validate_foreign_keys` violation branch**
  (`sqlite/backup.rs:200-212`) — re-covered by the dangling-FK negative (SQLite
  case); note DEC-C moves this call before `COMMIT`, so the surrounding rollback
  path is exercised too.
- **Missing-`db/` branch** (`backup.rs:204-209`) — re-covered by the
  missing-`db/` negative.

The plan may first attempt a clean public trigger for the export arm; cov:ignore
is the fallback for it only.

### Separable concern — issue #4 (plan's first task)

Expand issue #4 ("backup: restore drops the visibility tables") to cover
**both**: (a) add the missing visibility tables (`channels`,
`subscription_statuses`, `target_kinds`, `subscriptions`, `audiences`,
`audience_members`, `post_audiences`) to the backup table set, and (b) an
anti-regression framework — a check that enumerates the live schema's tables and
asserts each is either in `TABLES_IN_EXPORT_ORDER` or on an explicit exclusion
list, so a new migration that adds a table fails until its backup coverage is
decided. File via jaunder-issues as the plan's first task. #136 does **not**
touch product code; its tests keep the current author-viewer workaround until #4
lands.

### Decision record

Record, once ratified — a brief ADR draft or an amendment to the
still-`proposed` ADR-0053 (whose carve-out explicitly deferred backup test
_placement_ to #136): the backup-test-homing decision (DEC-A/DEC-B) **and** the
uniform restore-failure contract, including the SQLite validate-before-commit
rollback fix (DEC-C — a behavior change to `restore_backup`, so it warrants a
written rationale). A plan task.

## Acceptance criteria (observable)

- **AC1** — No DB-touching backup test remains in the storage crate:
  `rg '#\[apply\(sqlite_only\)\]' storage/src/backup.rs storage/src/sqlite/backup.rs`
  returns nothing; `migrated_pool`/`migrated_conn` are gone; the remaining pure
  `#[test]` helper tests in both files still pass.
- **AC2** — A dual-backend archive round-trip test exists and both cases pass.
- **AC3** — A dual-backend missing-`db/`-directory negative test exists: a
  backup with its `db/` removed fails to restore with `InvalidBackup` on both
  backends (covering `backup.rs:204-209`).
- **AC4** — A dual-backend malformed-row rollback negative test exists: a backup
  with a bad row in a non-first table (`posts`) fails to restore with
  `InvalidBackup` **and** an earlier table's rows (`users`) are absent from the
  target afterward, on both backends.
- **AC5** — After the DEC-C product change, a dual-backend dangling-FK negative
  test exists: restore fails with `BackupError::ConstraintViolation` **and**
  leaves the target unmodified on **both** backends (one shared assertion, no
  per-backend branch).
- **AC6** — The uniform restore-failure contract holds in code: Postgres restore
  maps SQLSTATE-`23` integrity failures to `ConstraintViolation`; SQLite restore
  runs `foreign_key_check` before `COMMIT` and rolls back on violation (verified
  by AC5 — the SQLite case asserts the target is unmodified, which fails on
  today's commit-then-check code).
- **AC7** — An A→B→A→B cross-backend cycle test exists (postgres_only): the
  fixture passes `assert_backup_fixture_restored` (now including the
  `published_at` **value**) at B, A₂, and B₂, and the two Postgres-side dumps
  are byte-identical (`E_B₁ == E_B₂`) over `db/*.ndjson` **and** `manifest.json`
  with only its `timestamp` field excluded (so
  `version`/`schema_version`/`schema_checksum`/`mode`/ `tables` are validated);
  it skips cleanly when Postgres testing is disabled.
- **AC8** — Value interop is asserted, not assumed:
  `assert_backup_fixture_restored` checks the restored `published_at` equals the
  fixed micro-precision value seeded by `populate_backup_fixture`, so every
  round-trip/hop/cycle test proves timestamp values survive.
- **AC9** — The two single-hop cross-backend tests still exist and pass.
- **AC10** — `populate_backup_fixture` / `assert_backup_fixture_restored` have a
  single definition, used by both `commands.rs` and `backup_interop.rs`.
- **AC11** — `cargo xtask validate --no-e2e` is green: static + clippy +
  coverage, no CRAP regression; the SQLite export-`Err` arm carries a new
  in-source `// cov:ignore` (reason), and the now-redundant Postgres
  restore-`Err` `// cov:ignore` is removed; `test_pattern_check` passes (no bare
  DB `#[tokio::test]`, every remaining single-backend test has a decisive
  `// reason:`).
- **AC12** — Issue #4 has an added scope comment covering the missing tables
  **and** the table-completeness anti-regression guard; no #4 product code
  changes land in #136.
- **AC13** — Decisions are recorded — a brief ADR draft in `docs/adr/drafts/`
  (or an amendment to ADR-0053): the backup-test-homing decision (DEC-A/DEC-B)
  and the uniform restore-failure contract + SQLite rollback fix (DEC-C) — once
  ratified at approval.

## Constraints & hazards for the plan

- **Coverage-neutral placement** (ADR-0053): one workspace-wide instrumented
  nextest pass with Postgres live; a test is instrumented on both backends
  wherever it lives.
- **Trigger errors through the public interface**: construct corrupt backups by
  tampering a real export, then drive `cmd_restore` / `restore_backup` — never
  by calling dialect internals.
- **SQLite TempDir / dual-pool hazards** (project memory): bind the whole
  `TestEnv`/base for the test's lifetime (dropping the base unlinks the SQLite
  file); the CLI pattern reopens a fresh pool per hop, sidestepping concurrent
  pools on one SQLite file — keep to that pattern.
- **Postgres gating**: cross-backend tests are `postgres_only` and early-return
  under `!postgres_testing_enabled()`, matching the existing interop tests.

## Out of scope

- Backup/restore **product** code beyond the DEC-C uniform-restore-failure
  change (the Postgres error mapping + SQLite validate-before-commit rollback).
  Everything else in the export/restore paths is unchanged.
- The `post_audiences` / visibility-table fidelity itself (issue #4).
- **Canonical cross-backend serialization** (DEC-D): timestamps (and other
  types) keep their per-backend text rendering on export, so cross-backend dump
  _bytes_ may differ. This is a documented cosmetic non-goal, not a defect —
  interop is proven at the value level instead. (Not filed as an issue per the
  decision.)
- Refactoring the dialect backup implementations (ADR-0019 keeps them separate).

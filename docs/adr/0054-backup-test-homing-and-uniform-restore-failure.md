# ADR-0054: Backup test homing and the uniform restore-failure contract

- Status: accepted
- Date: 2026-07-05
- Issue: [#136](https://github.com/jaunder-org/jaunder/issues/136)

## Context

ADR-0053 established storage test-homing and the dual-backend presumption, but
explicitly **deferred backup test placement to #136** (its "Backup carve-out"):
backup/restore is a cross-backend _contract_ (a portable dump), so it must be
tested through the public interface at the contract level — per-backend
round-trip fidelity, both-direction cross-backend portability, and a new double
round-trip.

#135 left 8 interim `#[apply(sqlite_only)]` backup tests in the storage crate
(`storage/src/backup.rs`, `storage/src/sqlite/backup.rs`) pointing at #136. In
resolving #136 two facts drove the decisions below:

1. The per-backend round-trip was **not missing** —
   `server/tests/misc/commands.rs` already ran it dual-backend at the
   CLI-command level (`cmd_restore_restores_directory_backup`,
   `#[apply(backends)]`), and covered every CLI-wrapper branch and restore
   precondition there. `cmd_backup`/`cmd_restore` are thin wrappers over
   `storage::export_backup`/`restore_backup`, so those CLI tests already
   exercise the storage orchestration on both backends. Rebuilding round-trips
   in the storage crate would only duplicate this at a lower altitude.
2. A constraint-violating restore failed **differently** on each backend: SQLite
   committed the import and then ran `PRAGMA foreign_key_check`, returning
   `ConstraintViolation` but leaving the invalid data committed; Postgres
   enforced the FK at insert time and surfaced a raw `sqlx::Error` (`Sqlx`),
   rolling back. Neither the error variant nor the post-failure state was
   uniform.

## Decision

**1. Home backup contract tests at the CLI/server-test level, not the storage
crate.** All backup fidelity and negative tests live in `server/tests/misc/`
(`commands.rs` for per-backend round-trips, negatives, and archive mode;
`backup_interop.rs` for cross-backend hops and the A→B→A→B cycle), driven
through `cmd_init`/`cmd_backup`/`cmd_restore` and asserted through `AppState`.
The 8 interim `sqlite_only` storage tests are deleted; their happy-path coverage
is subsumed by the CLI round-trip and transitive coverage, and their error-path
coverage is reconceived as dual-backend negatives (dangling-FK, malformed-row,
missing-`db/`). Placement is coverage-neutral (ADR-0053: one workspace-wide
instrumented pass with Postgres live).

**2. Keep both single-hop cross-backend tests and add an A→B→A→B cycle.** The
two existing hops (both directions) localize which direction broke; the cycle
(sqlite→postgres→sqlite→postgres) is an additional completeness proof. The cycle
asserts fixture **values** survive every hop and that the same-backend Postgres
dump pair is byte-identical (`E_B₁ == E_B₂`, over `db/*.ndjson` and
`manifest.json` minus its wall-clock `timestamp`). It does **not** assert
`E_A₁ == E_A₂`: SQLite writes `created_at`/`updated_at` at nanosecond precision,
which Postgres `timestamptz` quantizes to microseconds, so the SQLite dumps
differ byte-for-byte after a Postgres round-trip — a cosmetic difference, not a
fidelity loss.

**3. Restore fails uniformly across backends.** A constraint-violating restore
now returns `BackupError::ConstraintViolation` **and** leaves the target
unmodified on both backends: Postgres maps SQLSTATE class `23`
(integrity-constraint violations) to `ConstraintViolation`; SQLite runs
`foreign_key_check` _before_ `COMMIT` and rolls back on violation instead of
committing the invalid data first. This is a small, deliberate product change —
a uniform contract is the point of treating backup as a cross-backend contract —
and it fixes a latent SQLite integrity wart.

**4. Interop is value-level; no canonical serialization.** Cross-backend restore
already preserves timestamp _values_ (to Postgres's microsecond resolution);
#136 asserts that rather than assuming it. Cross-backend dump _bytes_ may differ
because each backend renders timestamps its own way. Canonicalizing that (as
booleans already are) is a deliberate non-goal.

## Consequences

- No `#[apply(sqlite_only)]` backup test remains in the storage crate; the
  `migrated_pool`/`migrated_conn` throwaway helpers are gone. The SQLite export
  rollback arm is `cov:ignore`d (unreachable through public `export_backup`,
  matching the Postgres arm); the Postgres restore rollback arm's `cov:ignore`
  is removed (now exercised by the negatives).
- `restore_backup`'s public error contract is now backend-uniform for constraint
  violations, and a rejected SQLite restore no longer leaves invalid data
  committed.
- The `post_audiences` / visibility-table backup gap remains out of scope,
  tracked in [#4](https://github.com/jaunder-org/jaunder/issues/4) (expanded to
  add the missing tables **and** a schema-completeness anti-regression guard).
  #136's tests keep the author-viewer workaround until #4 lands.
- Amends the ADR-0053 backup carve-out by settling the placement it deferred.

## References

- ADR-0053 — storage test homing and the dual-backend presumption (backup
  carve-out).
- ADR-0019 — backup excluded from the dialect dedup (dump/restore is
  backend-specific).
- ADR-0050 — stateless coverage gate (`cov:ignore` / CRAP).
- #136 — this reconception; #4 — the visibility-table backup gap.

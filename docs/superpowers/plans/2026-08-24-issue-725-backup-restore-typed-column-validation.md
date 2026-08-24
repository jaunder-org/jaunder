# Backup Restore Typed-Column Validation Implementation Outline

> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because restore validation is storage correctness work with cross-backend
> rollback and typed-column inventory invariants.

## Scope

In:

- Validate backed-up column values that already have Rust domain-value types
  during restore and report validation issues without refusing the restore.
- Preserve SQLite/Postgres backend parity while keeping ADR-0054 rollback
  behavior reserved for malformed backups and database integrity failures.
- Add a checked table/typed-column coverage guard so typed row validators cannot
  omit current backed-up domain columns or later be bypassed by primitive-only
  restore code.
- Cover `media.filename` as the concrete invalid-value regression.

Out:

- Backup manifest/schema-version compatibility changes; cross-version recovery
  for schema-compatible backups is tracked in #1161.
- Automatic repair, quarantine, or partial restore.
- New domain newtypes unrelated to columns already typed in Rust.
- Operator migration tooling for legacy backups.

## Task outline

- [x] Task 1: Build typed restore row validators
  - Contract: one shared restore-validation module maps backed-up tables to
    typed row validators. Each validator deserializes/reconstructs the table's
    domain fields through the Rust types that own their invariants, records any
    validation issues naming table, column, and value-class failure, and returns
    the original restore scalar values unchanged.
  - Contract: row validators are the validation mechanism; the checked
    table/typed-column inventory is the guard that proves the validators cover
    every current backed-up domain-value column, or records an explicit
    primitive-restore rationale.
  - Verification: focused host tests prove current backed-up domain-value
    columns are covered by a row validator, omission fails closed, and each
    covered column reaches the validation hook used before restore binding.

- [x] Task 2: Apply typed validation as restore diagnostics
  - Contract: SQLite and Postgres import paths call the shared row validator for
    each row before binding it; validation issues are accumulated in a restore
    report and do not become `BackupError::Sqlx` or `ConstraintViolation`.
  - Contract: validation runs before the backend commit point, but typed-domain
    validation issues do not trigger rollback; malformed NDJSON and database
    constraint failures keep the existing rollback behavior.
  - Verification: focused restore tests prove invalid `media.filename` reports
    on both backends while the database restore still completes.

- [x] Task 3: Prove the end-to-end restore-and-report contract
  - Contract: tamper a real directory-mode backup so `db/media.ndjson` contains
    raw `media.filename` such as `my photo.jpg`; leave backup media files
    present so full restore completion is observable.
  - Verification: `#[apply(backends)]` server/CLI-level test asserts restore
    completes with an unavoidable validation report naming `media.filename` and
    the filename value-class failure, target database contains the restored
    rows, and target media directory contains the restored media.
  - Verification: add or extend a happy-path restore assertion with a canonical
    percent-encoded filename such as `my%20photo.jpg`, proving encoded
    `media.filename` values restore and read normally on both backends.

## Risk checks

- A typed-domain validation issue must not be mapped to `ConstraintViolation`;
  database constraints and domain-value invariants are different failure
  classes.
- SQLite must still re-enable foreign keys after restore when validation issues
  were reported.
- Postgres must still commit restored data when the only problems are
  typed-domain validation issues.
- The coverage guard must be maintained from the actual backed-up
  schema/domain-type surface, not from the single `media.filename` regression
  fixture; row typing is the validation seam, the inventory is the completeness
  proof.
- `restore_backup` must still copy media only after database restore succeeds.
- Run focused restore tests first, then `devtool run -- cargo xtask precommit`
  before committing via `jaunder-commit`.

# Issue #725: Backup restore validates typed columns

## Outcome

Restoring a backup that contains a value violating a current domain-value
newtype invariant completes the restore but emits an unavoidable validation
report before the command finishes. The operator has the data on the new machine
and learns the bad table/column/value class at restore time instead of
discovering a later `sqlx` decode failure or silently missing row.

## Load-bearing decisions

- Restore is validated against the current binary and current schema invariants,
  but typed-domain validation is diagnostic for restore, not a gate that can
  strand a user without their data. A backup row whose stored value cannot
  construct the Rust type for that column is restored as stored and reported as
  a validation issue, even if later application reads may reject or skip that
  row until it is repaired by follow-up tooling.
- The chosen failure mode is restore-and-report. Typed-column validation issues
  do not roll back the database import, do not suppress media restore, and do
  not use ADR-0054's `ConstraintViolation` rollback contract, which remains
  reserved for database integrity failures and malformed backups that cannot be
  loaded faithfully.
- Domain validation issues are backup-content diagnostics, not infrastructure
  errors. They must not surface as a raw `sqlx::Error`, and they must identify
  the table and column that made restored data currently invalid under the Rust
  domain type.
- This issue does not remove the existing manifest version gate, add a backup
  schema-version policy, add an invariant-epoch migration system, or add repair
  tooling. Cross-version recovery for schema-compatible backups is tracked in
  #1161. Within backups the current restore path accepts, this fix prevents
  silent acceptance while keeping a decommissioning restore recoverable.
- Coverage uses `media.filename` as the concrete case from ADR-0084/#720:
  `Filename` is the canonical percent-encoded safe leaf stored in
  `media.filename`, and a raw display spelling such as `my photo.jpg` is not a
  valid stored filename.
- Restore validation should use typed table-row validation where the table has a
  coherent Rust row shape. A table validator may ignore primitive columns that
  have no domain invariant, but every backed-up column with an existing Rust
  domain type must be exercised through that type or carry an explicit
  primitive-restore rationale. The checked inventory exists to prove that the
  typed row validators cover the backed-up typed-column surface; it is not a
  hand-written per-cell validation substitute for row typing.

## Acceptance

- A directory-mode backup whose `media.ndjson` contains an unencoded
  `media.filename` value restores on both SQLite and Postgres and produces an
  unavoidable validation report naming `media.filename` plus the filename
  value-class failure, e.g. non-canonical stored filename.
- The validation report is produced during restore; it is not a later read-time
  decode error and not a generic database failure.
- After the restore with validation issues, the target database contains the
  restored rows and the target media directory contains the restored media on
  both backends.
- A valid backup containing a canonical encoded `media.filename` still restores
  and reads normally on both backends.
- The implementation includes a completeness guard for the typed-column
  inventory itself: a current backed-up column with a Rust domain-value type
  cannot be omitted silently, and an inventoried column cannot return to
  primitive-only restore binding with no newtype validation.

## Boundaries

- No quarantine/partial-restore mode; restore keeps the backed-up data together.
- No automatic rewrite of invalid backup values, including raw-to-encoded
  filename repair.
- No new backup manifest format, schema-version semantics, or operator migration
  command in this issue.
- No opportunistic redesign of unrelated domain invariants; the required audit
  is limited to columns in the backed-up schema that already have Rust
  domain-value types.

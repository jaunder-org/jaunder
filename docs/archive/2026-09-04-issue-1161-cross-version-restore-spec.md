# Issue #1161: Schema-compatible cross-version restore

## Outcome

An operator can restore a backup produced by another Jaunder release when the
current binary understands the backup format and the backup schema version
exactly matches the target schema. Package release chronology no longer strands
otherwise compatible data.

## Load-bearing decisions

- Restore compatibility has two authorities: backup format version and database
  schema version. The producing package version remains manifest provenance and
  never gates restore, whether it is older or newer than the current package.
- The manifest gains an integer `format_version` member. New exports write
  `format_version: 1`; a manifest without that member is legacy format 1, so
  every existing backup remains readable.
- An unsupported backup format fails before target mutation with a typed
  incompatibility error. Future representation changes that are not readable as
  format 1 must increment this version deliberately.
- Schema compatibility remains exact migration-version equality. A backup from
  an older package with the current schema is compatible; a backup from an older
  schema is not compatible until a separate change defines and tests an explicit
  migration path.
- Schema-version mismatch remains a typed incompatibility error and occurs
  before target mutation. It is distinct from malformed backup content and
  relational integrity failure.
- The existing schema checksum remains provenance and diagnostic metadata. It is
  not a compatibility gate because SQLite and PostgreSQL intentionally hash
  different schema representations while backups remain portable between them.
- Existing failure categories remain stable: malformed or unloadable content is
  an invalid backup, while database constraint failure is a constraint violation
  that rolls back the restore. The obsolete package-version mismatch category is
  removed.
- A harmless package-version difference produces no warning. Successful restore
  reports only actionable content diagnostics.
- Typed-domain invariant changes retain #725's restore-and-report contract: data
  is restored as stored and the unavoidable validation report identifies
  current-domain violations. Such diagnostics do not become compatibility
  failures and do not suppress media restoration.
- Restore retains strict empty-target preflight, authoritative transactional
  clear-then-load, deferred/final foreign-key validation, database-before-media
  ordering, and SQLite/PostgreSQL parity.
- The compatibility and failure policy is recorded in a new ADR and projected
  into `docs/ARCHITECTURE.md`. It introduces no new ubiquitous product term, so
  `CONTEXT.md` remains unchanged.

## Acceptance

- A newly exported manifest serializes `format_version` as the integer `1`.
- Both storage backends restore a legacy manifest with no format-version field
  when its schema version matches the target.
- Both storage backends restore supported-format, matching-schema backups whose
  producing package version is older or newer than the current package.
- A differing package version alone emits no warning or validation issue.
- An unsupported explicit format version fails with the format-incompatibility
  category before database mutation or media restoration.
- A schema-version mismatch fails with the schema-incompatibility category
  before database mutation or media restoration.
- Malformed backup content and relational constraint violations retain their
  existing distinct errors and rollback behavior.
- A matching-schema backup from another package version containing a current
  typed-domain invariant violation restores its database and media, then emits
  the existing validation report.
- Existing same-version directory, archive, and SQLite/PostgreSQL
  interoperability coverage continues to pass.
- Documentation states which manifest fields govern compatibility and why
  package version and schema checksum do not.

## Boundaries

- No support for restoring a different schema version and no compatibility
  registry, schema transformation, invariant epoch, or repair tooling.
- No structural trial restore as a substitute for an explicit compatibility
  decision.
- No change to backup table membership, serialization contents beyond the
  format-version field, restore ordering, target-emptiness policy, media layout,
  or cross-backend value-level fidelity.
- No warning, acknowledgement flag, or SemVer range policy for producing package
  versions.

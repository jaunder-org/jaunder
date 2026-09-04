# Cross-Version Restore Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for delegated tasks.
> This outline exists because the change introduces a durable backup-format
> boundary and alters storage restore compatibility policy.

## Scope

In:

- Version the existing backup manifest format without invalidating historical
  manifests.
- Replace package-version equality with supported-format and exact-schema gates.
- Preserve typed restore diagnostics and all transactional/backend invariants.
- Prove the public restore behavior through the established dual-backend CLI
  contract tests.
- Record and project the compatibility decision.

Out:

- Older-schema transforms, compatibility registries, repair tooling, structural
  trial restore, SemVer ranges, and operator acknowledgement.
- Changes to table membership, media layout, restore ordering, schema checksum
  calculation, or backup value serialization.

## Task outline

- [x] Task 1: Establish the versioned manifest compatibility contract
  - Contract: `manifest.json` writes integer `format_version: 1`;
    deserialization maps an absent member to v1. Package `version` remains
    serialized provenance. Restore validates supported format before exact
    schema version and no longer rejects package-version differences.
    Unsupported format and schema mismatch remain separate typed errors; package
    `VersionMismatch` is removed.
  - Verification: focused manifest serialization/deserialization and validation
    tests prove explicit v1 output, absent-field legacy input,
    unsupported-format rejection, exact-schema rejection, and ignored
    older/newer package versions.

- [x] Task 2: Prove restore behavior and backend parity at the contract boundary
  - Contract: use the public command/restore path and existing backup fixtures;
    both backends must observe identical compatibility, rollback, validation
    report, and media behavior. No storage-crate duplicate round-trip suite.
  - Verification: `#[apply(backends)]` tests cover legacy manifests; older and
    newer producing package versions with no warning or validation issue;
    unsupported format and schema mismatch before mutation/media; and
    differing-package restore with a typed-domain validation report. Existing
    malformed-content `InvalidBackup` and relational `ConstraintViolation` cases
    retain distinct errors and rollback assertions; directory, archive, and
    interop tests remain green.

- [x] Task 3: Record and project the restore compatibility decision
  - Contract: add proposed draft
    `docs/adr/drafts/backup-format-and-schema-compatibility.md`; cite existing
    numbered ADRs using the tracked-draft link convention. Update the backup
    section of `docs/ARCHITECTURE.md` with descriptive link text citing
    `docs/adr/drafts/backup-format-and-schema-compatibility.md` and the
    implemented format/schema/package/checksum authorities and failure taxonomy.
    Do not edit generated `docs/README.md` or `CONTEXT.md`.
  - Verification: documentation gates resolve every draft citation and the
    architecture prose agrees with the shipped manifest and errors.

- [x] Task 4: Make backup format review mechanically unavoidable
  - Contract: keep `CURRENT_BACKUP_FORMAT_VERSION` explicit and independent of
    migration count. Add a fail-closed host gate over the exact inventory of
    representation-defining backup modules. Compatible source drift requires
    refreshed Git blob identities; incompatible drift also requires a format
    version increment. Missing inputs, source-set drift, malformed inventory,
    and source/inventory version disagreement fail.
  - Verification: focused xtask tests cover clean inventory, source drift,
    source-set drift, malformed/non-literal version declarations, and version
    mismatch; the real-tree gate passes with the committed inventory.

## Ordering and contracts

- Task 1 owns the compatibility API and error taxonomy consumed by Task 2.
- Task 2 may extend fixtures but must not create a second compatibility policy;
  assertions follow Task 1’s public errors and the approved spec.
- Task 3 records the final names and behavior after Tasks 1–2 stabilize; the ADR
  remains numberless and proposed for post-merge promotion.
- Task 4 is a post-review hardening decision requested before PR finalization;
  it changes no restore behavior and keeps migration/schema authority separate.
- Each completed task proceeds through `jaunder-commit`; the commit hook owns
  the single precommit gate.

## Risk checks

- Missing `format_version` means v1 only; malformed values do not silently
  default, and unsupported explicit values fail closed.
- Compatibility checks and identity preflight finish before clear-then-load can
  mutate either backend; failed restores never restore media.
- Package version is removed only as a gate, not from manifest provenance or
  dump-stability comparisons.
- Schema checksum remains serialized and unchanged but is never compared across
  SQLite and PostgreSQL.
- #725 diagnostics remain nonfatal and unavoidable; malformed content and
  relational constraints retain their existing hard-failure categories.
- Search every constructor, pattern match, assertion, and user-facing message
  for the removed package-version mismatch variant.
- Backend-parametric tests and both-direction interoperability protect the
  portable-backup contract.
- The source inventory covers format, archive, media, catalog, binding,
  validation, orchestration, and both backend backup adapters exactly; moved or
  newly introduced representation logic requires an inventory change.

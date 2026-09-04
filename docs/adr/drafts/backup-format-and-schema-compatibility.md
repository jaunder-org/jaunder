# ADR-DRAFT: Backup format and schema compatibility

- Status: proposed
- Date: 2026-09-04
- Issue: [#1161](https://github.com/jaunder-org/jaunder/issues/1161)

## Context

A portable backup can outlive the Jaunder package that produced it. Treating the
producing package version as a restore gate strands data even when the receiving
release understands the backup representation and its database schema matches.
Conversely, attempting a restore merely because a package version appears close
does not establish that the manifest is readable or that its rows target the
current schema.

The manifest already carries package version and a backend-specific schema
checksum. They remain useful provenance for operators and diagnostics, but
neither can be a portable compatibility authority: package chronology does not
describe backup readability, and SQLite and PostgreSQL intentionally represent
and therefore checksum their schemas differently. Restore must retain the
uniform transactional failure contract of
[ADR-0054](../0054-backup-test-homing-and-uniform-restore-failure.md), the
strict empty-target and schema-derived backup policy of
[ADR-0064](../0064-backup-target-auto-derivation.md), and the clear-then-load
shape of [ADR-0115](../0115-clear-then-load-restore.md). Typed-domain invariant
violations also retain the diagnostic restore-and-report behavior established by
the
[archived #725 specification](../../archive/2026-08-24-issue-725-backup-restore-typed-column-validation-spec.md).

## Decision

Compatibility authorities are the backup format and the database schema version:

- The manifest carries an explicit integer `format_version`. Version 1 exports
  write `1`; an absent member in historical manifests means legacy format 1.
  Only supported format versions are readable.
- The format version is deliberately independent of database migration count:
  migrations already have the separate schema-version authority and do not all
  alter the portable representation. A fail-closed xtask inventory records the
  Git blob identity of every representation-defining backup source. Source drift
  must be acknowledged by refreshing the inventory; incompatible drift must also
  increment the explicit format version.
- A restore requires an exact schema-version match. No schema transformation or
  compatibility registry is implied.
- Producing package version and backend-specific schema checksum are provenance,
  not gates. Restores from older or newer packages that satisfy the format and
  schema authorities are accepted silently.
- Unsupported format and schema mismatch are typed incompatibility failures that
  occur before either database or media mutation. They remain distinct from
  malformed or unloadable backup content, and from relational constraint
  failure.
- Typed-domain invariant violations remain diagnostics after data and media have
  been restored; they are neither compatibility failures nor a reason to
  suppress media restoration.

## Consequences

Operators can recover schema-compatible backups across package releases without
warnings for harmless package-version differences, while unknown representations
and mismatched schemas fail closed before restore begins. The policy preserves
backend portability without pretending backend-specific schema checksums are
portable.

This decision rejects package-version equality, SemVer ranges, operator warning
or acknowledgement workflows, structural trial restores, migration-derived
format versions, and schema checksums as compatibility gates. Those alternatives
either confuse provenance with readability, impose arbitrary release chronology,
or make compatibility depend on backend-specific incidental representation. It
also rejects implicit cross-schema restore: supporting one requires a separately
specified and tested migration path.

The source inventory makes backup-format review unavoidable without pretending
that source identity can decide semantic compatibility. Compatible refactors may
refresh hashes at the current version; incompatible representation changes add a
version and retain the immutable legacy-v1 default.

# Issue #959 — split backup mechanics by concern

## Outcome

`storage::backup` exposes the same public and crate-internal behavior through
cohesive concern files instead of one mixed 1,489-line file. Backup bytes,
restore results, error mapping, media handling, backend behavior, and caller
paths remain unchanged.

## Load-bearing decisions

- `storage/src/backup.rs` becomes an assembly-only module root containing
  declarations and explicit reexports.
- `backup/orchestration.rs` owns `BackupExportOptions`, `BackupRestoreOptions`,
  `export_backup`, `restore_backup`, and backend/archive/media sequencing.
- `backup/error.rs` owns the cross-cutting public `BackupError` leaf used by
  orchestration, archive, media, format, restore validation, and backend
  modules.
- `backup/format.rs` owns `BackupManifest`, `ColumnInfo`, the derived table set,
  schema/version checks, manifest and NDJSON I/O, row scalar conversion, and
  deterministic ordering helpers.
- `backup/archive.rs` owns destination preconditions, temporary-directory
  lifecycle and cleanup reporting, tar creation, and tar extraction.
- `backup/media.rs` owns recursive media restore/mirroring, regular-file
  filtering, content comparison, hash calculation, previous-backup discovery,
  hard-link reuse, and copy fallback.
- Existing `backup/restore_validation.rs` remains the separate #725 typed
  restore-validation concern.
- Existing `storage::…` public exports, fields, variants, function signatures,
  and backend-visible crate interfaces remain stable through explicit reexports.
- SQLite and PostgreSQL backup implementations remain separate under ADR-0019;
  this change does not deduplicate their transaction, catalog, SQL, or sequence
  mechanics.
- Pure internal unit tests move with the concern they prove. Existing
  database-backed export/restore fidelity, negative, archive, and cross-backend
  contract tests are re-homed from `storage/src/backup.rs` to
  `server/tests/misc`, joining the contract suite required by ADR-0054.
- `docs/ARCHITECTURE.md` receives only source-path and symbol-citation
  corrections. No new ADR or unrelated #725 behavior documentation is added.

## Acceptance

- `backup.rs` contains only module declarations, explicit reexports, module
  documentation, and attributes.
- Each new implementation file has one named responsibility matching the
  decisions above; `restore_validation.rs` remains independent.
- Every public `storage` backup interface and every SQLite/PostgreSQL
  shared-helper import compiles unchanged from its caller's perspective.
- Existing manifest/table bytes, destination errors, temporary cleanup, archive
  layout, media recursion/deduplication, database-before-media restore order,
  constraint handling, and restore diagnostics retain their tests and behavior.
- Pure helper tests are co-located with their concern; no database-backed public
  backup contract test remains under `storage/src/backup/**`.
- No compatibility alias, duplicate implementation, dead path, or test-only
  forwarding shim remains.
- Live architecture citations identify the new owners; historical ADR and
  archive text remains historical.
- Focused storage and server backup tests pass, followed by `cargo xtask check`.

## Boundaries

- No backup format, schema, table membership, ordering, validation, failure,
  rollback, archive, media, or interoperability policy changes.
- No changes to backend-specific backup algorithms, server command behavior,
  migrations, public protocol, or persisted state.
- No line-count rule, generic framework, new trait seam, ADR, or #725
  documentation expansion is introduced.

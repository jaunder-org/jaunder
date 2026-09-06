# Backup format corpus maintenance

This directory is the checked-in, hand-authored evidence for backup _format_
compatibility. [`../backup_corpus.rs`](../backup_corpus.rs) verifies the
historical reader inputs before use; its test-owned raw-wire oracle separately
checks current writer output. Neither direction may derive its expected data
from the production encoder, manifest types/constants, serializers, or archive
helpers.

## Adding a format

A format bump is a compatibility decision, not a migration-count change. When a
reader or writer contract check reports backup-format incompatibility:

1. Write a new immutable directory fixture (for example, `format-2/`) by hand;
   do not regenerate, rewrite, or otherwise bless an existing fixture from
   production output.
2. Add exactly one `index.json` entry for its `format_version`, `support`, and
   normalized SHA-256 digest. A version may occur only once. The index is
   external metadata and is not part of a fixture digest.
3. Add the matching test-owned raw-wire oracle and reader/writer role inventory
   coverage. Cover the exact manifest member set and types, paths/table order,
   NDJSON object-per-line framing and trailing LF, applicable value roles and
   relationships, and media bytes. Keep explicit rationales for reader-only
   roles a current writer cannot emit.
4. Prove each supported fixture through public restore on both SQLite and
   PostgreSQL, as both a directory and an independently packaged archive. The
   archive must retain the fixture manifest's `mode`; tar/gzip bytes and
   metadata are not canonical.

The production writer must emit a version that resolves to exactly one supported
fixture and its matching oracle. Do not repair a format mismatch by editing
`format-1` history: add the new fixture, oracle, and index entry instead.

## Fixture integrity and schema materialization

Fixtures are immutable history. Before a temporary copy is made, the corpus
computes the digest over its regular files sorted by UTF-8 relative path with
`/` separators. Each path and its content is framed by its unsigned 64-bit
big-endian byte length followed by its bytes. Directories are traversed but not
hashed; symlinks and every special entry are rejected.

When adding a fixture, calculate its digest using that exact rule, put the
64-character hexadecimal value in its index entry, and review the fixture and
index together. A changed path or byte must produce a deliberate digest change.
For an already-recorded fixture, a digest mismatch is an investigation signal,
not an instruction to rehash history: preserve the fixture and find the
unintended mutation. The focused corpus integrity tests verify the checked-in
index digest and reject unsafe entries.

Restore has a separate, exact live-schema requirement. Tests first verify the
fixture, copy it to temporary storage, and change **only** the copied
`manifest.json` `schema_version` to the target's current version. The original
fixture, its provenance sentinels, and every other byte remain unchanged. This
is test materialization, not a schema migration or a format conversion.

## Retirement

Keep every historical fixture, including one for a retired format. After schema
materialization, the public restore matrix must still show supported entries
succeed and retired entries fail with the typed unsupported-format error before
database or media mutation. Retiring support requires an explicit approved
specification and ADR decision; changing `support` in `index.json` alone is
insufficient.

Format wire compatibility is intentionally distinct from live schema/migration
compatibility. Package version, timestamp, and backend-specific schema checksum
are provenance rather than format authorities. See the
[architecture projection](../../../../docs/ARCHITECTURE.md#backup-and-restore)
and
[ADR-0174](../../../../docs/adr/0174-backup-format-and-schema-compatibility.md).

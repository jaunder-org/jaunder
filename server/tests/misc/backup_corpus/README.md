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

1. Write a new immutable fixture tree by hand, then store it as a checked-in ZIP
   container (for example, `format-2.zip`); do not regenerate, rewrite, or
   otherwise bless an existing fixture from production output. Directory inputs
   are extracted from the ZIP during tests; ZIP is not a public restore format.
2. Add an `index.json` entry for its `format_version`, `support`,
   `materializable` role, and normalized SHA-256 digest. Each format version has
   exactly one current-schema materializable fixture; immutable
   historical-schema fixtures for that version may remain alongside it. The
   index is external metadata and is not part of a fixture digest.
3. Add the matching test-owned raw-wire oracle and reader/writer role inventory
   coverage. Cover the exact manifest member set and types, paths/table order,
   NDJSON object-per-line framing and trailing LF, applicable value roles and
   relationships, and media bytes. Keep explicit rationales for reader-only
   roles a current writer cannot emit.
4. Prove each supported fixture through public restore on both SQLite and
   PostgreSQL, as both an extracted directory and an independently packaged
   tar/gzip archive. The archive must retain the fixture manifest's `mode`;
   tar/gzip bytes and metadata are not canonical.

The production writer must emit a version that resolves to exactly one supported
fixture and its matching oracle. Do not repair a format mismatch by editing
`format-1.zip` history: add the new fixture, oracle, and index entry instead.

## Fixture integrity and schema materialization

Fixtures are immutable history stored as ZIP containers. Before extraction or
materialization, the corpus verifies a digest over regular files sorted by UTF-8
relative path with `/` separators. Each path and its content is framed by its
unsigned 64-bit big-endian byte length followed by its bytes. Directories are
traversed but not hashed; symlinks and every special entry are rejected.

When adding a fixture, calculate its digest using that exact rule, put the
64-character hexadecimal value in its index entry, and review the fixture and
index together. A changed path or byte must produce a deliberate digest change.
For an already-recorded fixture, a digest mismatch is an investigation signal,
not an instruction to rehash history: preserve the fixture and find the
unintended mutation. The focused corpus integrity tests verify the checked-in
index digest and reject unsafe filesystem entries and ZIP member paths/types.
The digest covers fixture paths and payload bytes, not ZIP container bytes.

Restore has a separate, exact live-schema requirement. A `materializable`
fixture is current-schema row-shape evidence: tests first verify it, extract it
to temporary storage, and change **only** the copied `manifest.json`
`schema_version` to the target's current version. The original fixture, its
provenance sentinels, and every other byte remain unchanged. This is test
materialization, not a schema migration or a format conversion.

A historical-schema fixture sets `materializable: false`. It remains immutable
format evidence, but its original schema version is restored unchanged and must
fail with the typed schema-mismatch error before mutation. There may be one
materializable fixture per format version plus any number of historical-schema
fixtures for that version.

## Schema-51 format-3 evidence

`format-3-schema-51.zip` contains hand-authored evidence derived from the
immutable schema-41 format-3 row examples, with a new provenance manifest and
explicit empty NDJSON members for the drained code-migration queue and five
system-theme tables. It models an initialized, never-served instance:
release-role installation belongs to startup admission. The old `format-3.zip`
payloads and digest remain unchanged and its index entry is historical-schema
evidence, not current-schema materialization.

The schema-51 evidence extends the independent format-3 table inventory and
retains its exact schema version and complete path set. Its populated writer
seed installs canonical A, B-app and B-theme inventories; the raw-wire checks
require current application/theme identities, binary manifest cells,
live/retained references and six exact immutable content members. The public
backup interoperability tests and canonical styling qualification separately
prove populated restoration/readability. Neither proof relabels a historical
fixture or changes the format version.

## Schema-52 format-3 evidence

`format-3-schema-52/` extends the hand-authored schema-51 examples with three
Session rows whose identical labels deliberately carry distinct `browser`,
`machine`, and `unknown` kinds. Migration 0052 adds that closed text field, not
a new table or backup format. Its separate provenance manifest pins schema 52.
The schema-51 fixture and digest remain unchanged; its index entry is now
historical-schema admission evidence and must be rejected before mutation.

The current-writer oracle explicitly requires schema 52 and the unchanged
independent table/path inventory, plus exact Session wire members, owner, shared
label and all three stored kinds. The public reader matrix restores the new
immutable fixture through directory and archive inputs on both backends and
checks those kinds via Session storage. Formats 1 and 2 retain their historical
row-decode coverage. The separate Session codec test retains absent-kind to
Unknown decoding; exact-schema admission still precedes row loading. No fixture
is upgraded in place.

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

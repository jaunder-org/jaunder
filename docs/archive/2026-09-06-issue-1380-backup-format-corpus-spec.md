# Issue #1380 — Behavioral backup format compatibility corpus

## Outcome

Backup format compatibility is proven against an independent behavioral corpus,
so production readers and writers cannot drift together unnoticed. Comments and
behavior-preserving refactors do not affect the evidence.

## Load-bearing decisions

- The corpus verifies both directions: hand-authored historical backup data is
  restored by the production reader, and production exports are checked by a
  test-owned raw-wire oracle. The oracle inspects bytes and untyped JSON; it
  does not use production manifest types, format constants, serializers, archive
  helpers, or encoder-derived expected values.
- Each format has an immutable, checked-in directory fixture and a corpus-index
  entry recording its version, support state, and normalized tree digest. The
  digest is SHA-256 over regular files only, ordered by UTF-8 relative paths
  with `/` separators. Each path and content is framed by its unsigned 64-bit
  big-endian byte length followed by its bytes. Directories are traversed but do
  not enter the digest; symlinks and other special entries are rejected, and the
  external corpus index is excluded.
- Historical fixtures are never regenerated from production output. Adding a
  format adds a fixture; changing support policy updates the index and the
  governing decision, not the historical fixture.
- The historical v1 fixture omits `format_version`, uses a package-version
  sentinel unequal to the test binary, and uses a schema-checksum sentinel
  unequal to either target backend. Current production exports must instead
  contain the integer `format_version: 1`.
- Public restore requires the current exact database schema version. Tests copy
  a fixture to temporary storage, verify its digest first, and replace only its
  manifest `schema_version` with the target's current version. No production
  compatibility gate or migration metadata is bypassed or weakened.
- Fixtures use `instance_identity` plus the smallest stable set of tables and
  rows needed by a test-owned reader-role inventory. The inventory names each
  fixture path, table, column, exact wire value, and expected restored value or
  relationship for null, boolean, integer, real, text, structured JSON,
  relationships, and media.
- A separate writer-role inventory names seeded sources for every role the live
  schema can emit and records why non-emittable reader roles, including real and
  structured JSON values, are inapplicable. Existing live-schema tests retain
  complete table and current-domain coverage.
- Every supported fixture is restored as a directory and as an archive on both
  SQLite and PostgreSQL. Archive input is packaged independently from the
  checked-in directory and retains the fixture's manifest `mode`; only
  `schema_version` changes. Compressed bytes and tar metadata are not canonical.
- Format-owned structure is exact: relative paths, manifest member names and
  types, manifest table ordering, NDJSON one-object-per-line framing and
  trailing newline, and media bytes. Row key order, insignificant JSON
  whitespace, backend numeric spelling, and restored values are compared
  semantically.
- Package version, timestamp, and backend-specific schema checksum remain
  provenance. The reference validates their presence and types but does not make
  their values compatibility authorities.
- The writer check seeds known live data and applies its raw-wire oracle to both
  production directory exports and independently extracted production archive
  exports on both backends. The oracle owns the exact manifest key set, literal
  format version, path set, table ordering, NDJSON framing, role inventory, and
  expected values. Same-code round trips and production-generated golden files
  are not substitutes for this check.
- Fixtures remain present for retired formats. After schema materialization,
  every index entry is dispatched through public restore: supported formats
  succeed, while unsupported formats return the typed unsupported-format error
  before database or media mutation. The production writer's emitted version
  must resolve to exactly one supported entry and its matching oracle. Retiring
  support requires an explicit specification and ADR decision.
- ADR-0174 already owns backup format and schema compatibility and identifies
  this corpus as its independent enforcement follow-up. This work updates the
  architecture projection and fixture-authoring documentation; it does not
  create a second compatibility ADR or edit the generated ADR index.
- No ubiquitous product term is introduced, so `CONTEXT.md` remains unchanged.

## Acceptance

- The checked-in historical v1 fixture omits `format_version`, carries package
  and checksum sentinels, and passes its normalized tree-digest check before
  test-only materialization.
- Changing a fixture path or byte without updating its index digest fails;
  ambiguous path/content framing, symlinks, and special entries are rejected
  while ordinary directories are traversed.
- After materialization, the copied tree differs from the verified fixture only
  in the manifest's `schema_version` value.
- Format 1 restores successfully for all four
  `{SQLite, PostgreSQL} × {directory, archive}` combinations through the public
  restore command despite the legacy field omission and provenance sentinels.
- The reader-role inventory is exhausted in each restore combination, asserting
  every wire value, restored value or relationship, and exact media bytes.
- Production directory and archive exports on both backends satisfy the
  independent v1 manifest, path, NDJSON framing, applicable writer-role, and
  media contract; archives are extracted without production archive helpers, and
  every reader-only role has an explicit inapplicability rationale.
- Current exports contain an integer `format_version: 1`. The raw-wire oracle
  rejects missing or extra manifest members while comparing backend-variable row
  values semantically.
- Every corpus-index entry agrees with public restore dispatch: supported
  versions succeed, unsupported versions fail before mutation with the typed
  unsupported-format error, and the writer emits exactly one supported version.
- Existing malformed-manifest, malformed-row, constraint rollback, full table
  membership, archive, and backend-interoperability tests remain green.
- Documentation identifies the corpus as the format-version enforcement
  mechanism, explains how to add immutable fixtures and support-state entries,
  and keeps migration/schema compatibility separate.
- Reader- or writer-contract mismatches identify backup format compatibility as
  the failed invariant and direct authors to introduce a new format fixture,
  oracle, and corpus-index entry instead of changing an existing fixture.
- `docs/ARCHITECTURE.md` projects the delivered corpus enforcement policy under
  ADR-0174 without changing that accepted decision's status or generated index
  row.

## Boundaries

- No source-file hash inventory or format version derived from migration count.
- No production encoder used to generate or bless historical expected data.
- No production manifest type, format constant, serializer, archive helper, or
  decoder reused as the independent writer oracle.
- No promise that tar/gzip bytes, JSON object key order, numeric spelling,
  package version, timestamp, or schema checksum are canonical across backends.
- No duplicate malformed or corrupt backup corpus; existing negative tests
  retain that responsibility.
- No production multi-version decoder registry, schema migration support, or
  change to restore ordering, transactionality, media layout, or table policy.

# Behavioral Backup Format Corpus Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for each bounded
> slice. This outline exists because issue #1380 introduces durable
> backup-format evidence across storage backends and archive modes.

Authoritative spec:
`docs/superpowers/specs/2026-09-06-issue-1380-backup-format-corpus.md`

## Scope

In:

- Checked-in immutable format fixtures and an external support/digest index.
- Test-owned raw-wire inspection, independent archive packaging/extraction,
  reader and writer role inventories, and public restore/export proofs.
- Corpus-authoring documentation and the ADR-0174 architecture projection.

Out:

- Production backup representation, version dispatch, restore ordering, schema
  policy, migrations, media policy, and existing negative-test responsibilities.
- A new ADR, generated ADR-index edits, or production-generated historical
  fixtures.

## Module and seam contracts

- Home the corpus contract under `server/tests/misc/`, as ADR-0054 requires. If
  a directory module is used, its `mod.rs` remains assembly-only.
- Checked-in assets live under one test-owned corpus root with an external
  index. The index is not part of any fixture digest.
- One test-only corpus interface owns index parsing, normalized digest
  verification, safe fixture copying, dynamic `schema_version` materialization,
  and independent archive packaging/extraction. Reader and writer tests consume
  that interface rather than reimplementing filesystem rules.
- One test-owned raw-wire oracle owns exact manifest/path/table/NDJSON/media
  inspection and format-bump diagnostics. It accepts raw paths and bytes, never
  production backup manifest/types/constants/serializers/archive
  helpers/decoders.
- Inventories are explicit data beside the oracle: the reader inventory maps
  exact fixture wire roles to restored values or relationships; the writer
  inventory maps seeded sources to applicable emitted roles and reader-only
  inapplicability reasons.

## Task outline

- [x] Task 1: Establish the immutable format-1 corpus and prove its integrity
      contract.
  - Contract: hand-authored legacy-v1 directory fixture, external index with
    support state and normalized digest, deterministic length-framed hashing,
    traversal-directory allowance, rejection of symlinks/special entries, and
    materialization that changes only `schema_version` after verification.
  - Verification: focused `misc::backup_corpus` integrity tests demonstrate
    digest sensitivity, unambiguous framing, entry-kind rejection, sentinel
    preservation, and the sole permitted materialization delta.

- [ ] Task 2: Prove historical reader compatibility through every public restore
      path.
  - Contract: the Task 1 interface independently packages archive input; public
    `cmd_restore` consumes both directory and archive forms on SQLite and
    PostgreSQL for every supported fixture; the reader-role inventory is
    exhausted and asserts exact restored values, relationships, and media bytes.
    Every index entry is materialized and dispatched through public restore:
    supported entries succeed, while retired or otherwise unsupported entries
    return the typed unsupported-format error before database or media mutation.
  - Verification: one backend × input-mode matrix under `misc::backup_corpus`,
    plus index-wide dispatch assertions covering every entry's declared support
    state.

- [ ] Task 3: Prove current writer compatibility with the independent raw-wire
      oracle.
  - Contract: production `cmd_backup` directory and archive outputs from both
    backends are inspected without production format or archive helpers;
    independently extracted archives satisfy the same v1 oracle as directories;
    exact manifest keys/types/version, path set, alphabetical table list, NDJSON
    framing, applicable role values, and media bytes are enforced while
    backend-variable numeric spelling and provenance values are semantic/type
    checks.
  - Verification: one backend × output-mode matrix exhausts the writer-role
    inventory, records every reader-only inapplicability rationale, emits
    format-bump guidance for contract mismatches, and proves the emitted version
    resolves to exactly one supported index entry and its matching oracle. After
    the combined reader/writer contract is green, rerun the retained
    malformed-manifest, malformed-row, constraint-rollback, full-table, archive,
    and backend-interoperability suites through their existing focused filters.

- [ ] Task 4: Publish the corpus maintenance and architecture contract.
  - Contract: test-adjacent documentation explains adding a new immutable
    fixture/index/oracle entry without regenerating history, digest refresh
    review, and format-bump diagnostics. It states that retiring support
    preserves the historical fixture and requires an explicit specification and
    ADR decision rather than an index-only edit. `docs/ARCHITECTURE.md` names
    the corpus as ADR-0174's enforcement mechanism and keeps format
    compatibility separate from schema/migration compatibility.
  - Verification: documentation/link/architecture parity checks selected by
    `jaunder-iterate`; no ADR or generated `docs/README.md` change.

## Ordering and handoff

- Task 1 fixes the corpus filesystem/index interface before reader and writer
  work.
- Task 2 consumes Task 1's materialized fixtures and independent archive
  packager.
- Task 3 consumes Task 1's corpus interface but owns the raw-wire oracle and
  writer inventory; it may reuse the independent extractor, not production
  archive code.
- Task 4 follows the delivered interface and diagnostics so its instructions
  describe the actual maintenance workflow.
- Each completed task updates its checkbox before its `jaunder-commit` gate.

## Risk checks

- Keep the historical fixture byte-authored and immutable; tests may copy and
  materialize it but never write back or bless it from production output.
- Derive only the temporary manifest's current schema version through existing
  public test setup; do not hardcode migration 32 or weaken exact-schema
  restore.
- Keep digest and oracle expectations independent from production constants and
  types, including literal format version `1` and manifest member names.
- Do not treat tar/gzip metadata, JSON key order, numeric spelling, timestamps,
  package version, or schema checksum as cross-backend canonical bytes.
- Preserve current malformed-manifest, malformed-row, rollback, full-table,
  archive, and interop tests rather than duplicating their scope.
- Keep backend parity explicit through the repository's dual-backend test
  templates and public CLI command seam.
- Ensure mismatch messages name backup format compatibility and direct authors
  to add a new fixture, oracle, and index entry rather than editing a historical
  fixture.

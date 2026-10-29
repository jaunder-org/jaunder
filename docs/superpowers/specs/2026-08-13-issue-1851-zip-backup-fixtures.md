# Issue #1851: Store backup corpus fixtures as ZIP archives

## Outcome

Represent all six existing backup compatibility fixtures as checked-in ZIPs
instead of checked-in directory trees, reducing review noise from dozens of
fixture files. Directory restore inputs are extracted from those archives at
test time; fixture payloads and test coverage remain unchanged.

## Approach and tradeoffs

Convert the six indexed fixtures (formats 1–3, each historical-schema and
current-schema) into checked-in ZIP containers while preserving every existing
file path and payload byte. Do not regenerate historical evidence with the
current production writer. ZIP is only the checked-in fixture representation; it
is not a public restore archive format. Extract each ZIP to materialize fixture
trees for directory restore cases and independently package
extracted/materialized trees as the existing tar/gzip (.tar.gz) inputs for
public archive restore cases. Keep this test packaging independent of production
serialization and packaging helpers. Update the corpus index and maintenance
README to describe the ZIP source-of-truth and extraction workflow.

Preserve the test-owned independent expected values, raw-wire writer oracle, all
six existing normalized payload digests, and integrity behavior, including
schema-only temporary materialization and rejection of unsafe archive entries.
This changes storage and materialization of fixtures, not backup compatibility
behavior or production formats. ZIPs reduce the expanded tracked diff; restore
tests continue to exercise directory and existing tar/gzip archive inputs.

## Acceptance

- All six currently indexed fixtures are represented by checked-in ZIPs, with
  exact historical paths and payload bytes preserved.
- Directory test inputs are extracted from those ZIPs; archive restore inputs
  are independently packaged as the existing tar/gzip (.tar.gz) format, not ZIP.
- Existing independent historical expectations and current-writer oracle remain
  intact and are not generated from production output.
- All six normalized payload digests remain unchanged; schema-only temporary
  materialization and unsafe-entry rejection remain integrity behavior.
- Restore coverage remains for directory and tar/gzip archive inputs on both
  SQLite and PostgreSQL, with existing supported/historical schema behavior and
  integrity checks preserved.
- The corpus index and README accurately describe fixture identity, archive use,
  and extraction; focused corpus tests pass on both backends.

## Boundaries

Do not remove tests or expectations, weaken backend parity, add ZIP restore
support, change existing backup formats or restore semantics, regenerate
historical payloads, or broaden this into a backup architecture redesign. The
goal is less PR review clutter, not fewer tests.

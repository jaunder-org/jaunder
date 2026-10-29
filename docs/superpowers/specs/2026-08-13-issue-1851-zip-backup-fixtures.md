# Issue #1851: Store backup corpus fixtures as ZIP archives

## Outcome

Represent the six original backup compatibility fixtures and the upstream
schema-52 format-3 fixture as seven checked-in ZIPs instead of expanded
directory trees, reducing review noise from dozens of fixture files. Directory
restore inputs are extracted from those archives at test time; all fixture
payloads, normalized digests, and test coverage remain unchanged.

## Approach and tradeoffs

Keep the six original indexed ZIP fixtures byte-identical. Convert the upstream
`format-3-schema-52` directory into `format-3-schema-52.zip`, preserving its 51
relative file paths, every payload byte, and normalized SHA-256 digest
`c9433574d39304d2e08dc9452ba9989bba824e7ac1dea0f2ef74b464bb7967c1`. Do not
regenerate historical evidence with the current production writer. ZIP is only
the checked-in fixture representation; it is not a public restore archive
format. Extract each ZIP to materialize fixture trees for directory restore
cases and independently package extracted/materialized trees as the existing
tar/gzip (.tar.gz) inputs for public archive restore cases. Keep this test
packaging independent of production serialization and packaging helpers. Update
the corpus index and maintenance README to describe the ZIP source-of-truth and
extraction workflow.

Preserve the test-owned independent expected values, raw-wire writer oracle, all
six existing normalized payload digests, and integrity behavior, including
schema-only temporary materialization and rejection of unsafe archive entries.
This changes storage and materialization of fixtures, not backup compatibility
behavior or production formats. ZIPs reduce the expanded tracked diff; restore
tests continue to exercise directory and existing tar/gzip archive inputs.

## Acceptance

- All seven indexed fixtures are represented by checked-in ZIPs: the six
  original archives remain byte-identical, and the upstream schema-52 directory
  is converted to a ZIP with its exact 51 paths, payload bytes, and normalized
  digest preserved.
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

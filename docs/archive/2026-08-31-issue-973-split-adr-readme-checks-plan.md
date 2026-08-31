# Split ADR README checks implementation outline

**Execution:** `jaunder-iterate`, delegating leaf ownership through
`jaunder-dispatch`.

## Trigger and scope

The approved issue #973 spec is a local behavior-preserving refactor, but the
three independent leaves will be extracted in parallel. This outline fixes the
shared private interfaces and integration boundary so agents do not invent
competing surfaces.

In scope: `xtask/src/adr_readme.rs` to wiring-only directory module plus three
private leaves, moved unit tests, and current `docs/ARCHITECTURE.md` source-path
citations. Out of scope: ADR/status/table/view behavior, CLI or gate changes,
#742 promotion redesign, historical-document rewrites, and #1169.

## Tasks

- [x] **Extract ADR file ownership** — create `adr_readme/files.rs` with the
      single numbered-ADR inventory, models and status/heading parsing, format
      validation, and their existing tests.
- [x] **Extract README projection ownership** — create `adr_readme/readme.rs`
      with table parsing/rendering/splicing, synchronization/parity, markers,
      and their existing tests.
- [x] **Extract architecture-view ownership** — create `adr_readme/view.rs` with
      accepted-ADR citation parity and its existing tests; update only current
      architecture source citations whose paths moved.
- [x] **Assemble and verify the facade** — replace `adr_readme.rs` with
      wiring-only `adr_readme/mod.rs`, explicitly re-export the unchanged
      surface, resolve only extraction-induced visibility/import/test-fixture
      seams, and verify focused xtask behavior plus the test-enabled repository
      gate.

## Stable contracts

- `files.rs` owns and exposes to the facade the existing `ADR_DIR`, `AdrEntry`,
  `parse_adr_dir`, and `format_problems`; it also exposes the existing
  `PROPOSED`, `ACCEPTED`, `StatusLine`, and `status_line` at their current
  crate-visible contract for `adr.rs`.
- `readme.rs` owns and exposes to the facade the existing `README`, `BEGIN`,
  `END`, `TableRow`, `parse_table_block`, `render_block`, `splice_block`,
  `sync_readme_at`, `readme_has_markers`, `sync_readme`, `parity_problems`, and
  `parity_report`.
- `view.rs` owns and exposes to the facade the existing `VIEW` and
  `view_parity_problems`.
- Private sibling dependencies use `super::files` / `super::readme` directly;
  they do not create new facade exports. `files.rs` owns the shared inventory
  seam: `AdrEntry` and `parse_adr_dir` at their existing facade visibility, plus
  sibling-visible `AdrFile`, `adr_files_from`, and `parse_adr_files` for README
  population and the retained fail-closed injected-file-type test. `readme.rs`
  owns sibling-visible `parity_report_with` and that cross-concern test.
  `files.rs` also owns the single sibling-visible `adr_link` formatter; both
  `readme.rs` and `view.rs` import it rather than duplicate link policy.
- `mod.rs` contains documentation, private `mod` declarations, and explicit
  `pub`/`pub(crate)` re-exports only. Existing callers remain untouched.
- Parallel leaf tasks create distinct files and do not modify/delete the source
  monolith or facade. The integration task alone removes `adr_readme.rs`,
  creates `mod.rs`, and resolves cross-leaf imports.
- Existing test names/assertions/fixtures move unchanged. A fixture may be
  duplicated privately when that keeps leaves independent; no test-support
  module is introduced solely for this split.

## Risk checks and verification

- Confirm the three gate steps retain order, names, details, and recovery text.
- Confirm promotion still rewrites the exact status span and synchronizes the
  README projection; preserve fail-closed unreadable-file behavior.
- Run focused xtask tests with the required xtask manifest path, then
  `devtool run -- cargo xtask check`; the commit hook owns the final precommit
  gate. No lint suppression is expected or approved.

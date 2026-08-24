# Issue 703: doc-links resolves targets against tracked content

## Outcome

`doc-links` reports a tracked Markdown link as live only when its target is
present in Git's tracked set, matching the file enumeration rule the gate
already uses. A link to an untracked or gitignored file must fail locally the
same way it fails in a fresh checkout or CI.

## Load-bearing decisions

- The gate's definition of existence is Git-tracked repository content, not the
  developer's working-tree filesystem.
- Directory links remain valid when the target directory contains at least one
  tracked path; this preserves existing `adr/`-style directory links without
  making untracked directories count as live content.
- Tracked-but-absent paths keep their current behavior as gated files: they are
  skipped as inputs, so staged deletions fail through Git/state reconciliation
  rather than as link-target noise.
- Fragment handling stays unchanged: strip `#fragment` before target lookup and
  do not validate anchors.
- Link syntax stays unchanged: inline Markdown links only, outside fenced blocks
  and inline code spans; no reference-style link, HTML link, title,
  percent-encoding, or anchor-validation expansion.
- Scope exclusions stay unchanged: `docs/archive/` and `docs/superpowers/` are
  excluded as gated files. If a checked document links into those excluded
  trees, the target still must be tracked to count as live.
- `xtask::doc_links::dead_links_in` remains the single resolver used by both
  `adr promote`'s warning path and the `doc-links` hard gate, so warning
  semantics and gate semantics cannot drift.
- No ADR is added. This is a correction to the #682 gate semantics already
  recorded in issue #682 and the architecture view, not a new architecture
  boundary.

## Acceptance

- A test with tracked Markdown linking to an untracked existing file reports a
  dead link with the existing `<file>:<line> -> <target>` shape.
- A test with tracked Markdown linking to a gitignored existing file reports a
  dead link.
- A test with tracked Markdown linking to a directory containing tracked files
  passes; the same shape for a directory containing only untracked content
  fails.
- Existing exclusions and parser rules are still covered: ignored gated files
  under `docs/archive/` / `docs/superpowers/`, code spans, fenced code blocks,
  bare anchors, external URLs, and tracked-but-deleted input files keep their
  previous behavior.
- The real repository passes the focused `doc-links` coverage under the xtask
  test suite and the normal `check --no-test` gate.
- Documentation that describes `doc-links` target resolution no longer says
  targets are resolved with `.exists()` against the working tree; it states the
  tracked-set rule and the directory exception.

## Boundaries

- Do not validate Markdown anchors.
- Do not add a new `cargo xtask` subcommand or alternate CLI surface.
- Do not widen the checked file set.
- Do not make `doc-links` inspect untracked documents.
- Do not change ADR promotion's path rewriting rules except for inheriting the
  corrected shared target resolver.

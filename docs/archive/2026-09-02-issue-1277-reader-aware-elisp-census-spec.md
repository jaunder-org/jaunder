# Reader-Aware Elisp Census

Issue: #1277 Status: Approved

## Outcome

The census accepts reader-valid Emacs Lisp regardless of parentheses inside
strings or comments, extracts only actual top-level `require` forms, and still
fails explicitly for malformed source. Clone-shape and dependency collectors
share one owned Emacs-reader boundary instead of maintaining separate process
lifecycles or partial grammars.

## Diagnosed failure

- `cargo xtask census --json` reported
  `elisp/test/jaunder-pull-media-test.el: unbalanced Elisp forms` even though
  Emacs reached point-max at depth zero and read all top-level forms.
- `balanced_elisp` counts raw `(` and `)` characters, including characters that
  the Emacs reader treats as string or comment content.
- The dependency collector gates its line-based `require` scan on that raw
  count, so a false structural verdict fails the entire dependency cell.
- The clone collector already proves the repository can use the actual Emacs
  reader with bounded diagnostics and explicit unavailable-versus-failed
  outcomes.

## Load-bearing decisions

- A shared `census::elisp` module owns Emacs invocation, reader protocols, and
  reader error classification while reusing the existing bounded drain and
  cleanup utilities in `census::process`.
- The module exposes specialized function-shape and top-level-dependency
  operations; embedded Elisp programs and output protocols remain private.
- Each source file is read in its own invocation, preserving path-local failure
  reporting and avoiding a new multi-file framing protocol.
- Dependency extraction and structural validation happen in the same
  actual-reader pass. Strings, comments, nested forms, and multiline formatting
  are not interpreted by a Rust line scanner.
- Only reader-observed top-level `(require 'feature)` forms contribute
  dependency identities; dynamic and nested loading remains excluded.
- The clone collector migrates cleanly to the shared owner, deleting its private
  Emacs process implementation.
- The dependency collector retains an injected reader seam for deterministic
  result classification tests.
- A missing Emacs executable yields an unavailable Elisp dependency cell.
  Malformed source, reader non-zero exit, or reader I/O failure yields a failed
  cell with source-path context and retained diagnostics.
- No fallback parser or raw parenthesis precheck remains.

## Acceptance

- Reader-valid Elisp containing unmatched-looking parentheses in both a string
  and a comment is accepted by the dependency collector.
- Actual top-level quoted `require` forms are extracted, including reader-valid
  multiline forms.
- Require-shaped text in strings or comments and nested `require` forms do not
  become top-level dependency evidence.
- Genuinely malformed Elisp produces a failed dependency cell naming the source
  path.
- A deterministically unavailable injected reader produces an unavailable
  dependency cell rather than clean or failed evidence.
- Existing clone actual-reader normalized-shape and injected-unavailable tests
  pass after the clean cutover; the shared implementation retains the clone
  collector's existing drain and cleanup paths without redesigning them.
- The raw `balanced_elisp` helper and dependency line scanner are deleted when
  no callers remain.
- Focused xtask census tests pass.
- A full `cargo xtask census --json` completes without failing on
  `elisp/test/jaunder-pull-media-test.el`; genuine malformed fixtures remain
  failed.
- The normal pre-commit and CI gates pass.

## Boundaries

- Do not change census signal ordering, candidate identity formats, report
  schema, command failure policy, source snapshot selection, or non-Elisp
  collectors.
- Do not broaden dependency evidence beyond literal top-level quoted `require`
  forms.
- Do not add an Elisp parser dependency, invoke a shell, or retain a fallback
  partial grammar.
- Do not change production Emacs client code or tests outside the focused census
  collector coverage required by this defect.
- No ADR is required: this deepens the existing declared reader boundary without
  changing a durable external architecture or protocol.

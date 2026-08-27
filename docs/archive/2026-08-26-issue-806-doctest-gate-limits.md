# Issue #806 — Close the doctest gate's known limits

## Outcome

The doctest gate enumerates CommonMark fenced blocks correctly, reconciles every
reported run entry deterministically, and reports unreadable inputs without a
fake source line. The host check runs auxiliary-workspace doctests exactly once
and behaves identically when invoked outside the repository root.

## Load-bearing decisions

- Fence recognition implements the relevant CommonMark grammar rather than a
  doctest-specific approximation: an opener is at most three spaces indented,
  uses at least three homogeneous backticks or tildes, and may carry info text.
  Backtick-fence info text cannot contain a backtick.
- A closer is at most three spaces indented, uses the opener's marker kind with
  at least the opener's length, and has only spaces or tabs after the marker.
  Other marker-looking lines remain content. An unclosed fence extends to the
  end of its doc block.
- The closed doctest info vocabulary remains unchanged: plain Rust,
  `compile_fail`, and `text` after the existing whitespace normalization. Fence
  grammar support does not admit a fourth category or weaken fail-closed
  reconciliation.
- Run entries are grouped by exact `(file, line)` key. More than one entry at a
  key always emits one duplicate-key violation, including identical duplicates.
  The same group independently folds its result: any failed entry is `Failed`;
  otherwise all ignored is `NotRun`; otherwise the fence ran. Input order cannot
  change violations. A duplicate group with no scanned Rust fence, including a
  group matching `text`, emits one `Orphan` per key rather than per entry.
- `Violation.line` is optional. Located fence, run, and parse violations carry a
  line; unreadable-file violations carry none, serialize as `"line": null`, and
  render as `<path> [unreadable]` without `:0`.
- The host test step selects only library, binary, and integration-test targets
  for the `xtask` and `tools` workspaces. The doctest-fences step remains the
  sole host `--doc` execution because it owns the captured output needed for
  reconciliation.
- The host doctest-fences runner resolves the Git top-level once and derives
  absolute source and manifest paths from it. Invocation cwd does not affect the
  scanned population or commands. The Nix/devtool producer keeps its intentional
  derivation-source cwd contract.
- This refines ADR-0095's existing parser/reconciler mechanism. It introduces no
  new gate exemption, vocabulary category, domain term, or architectural
  decision.

## Acceptance

- A longer outer backtick or tilde fence can contain shorter and wrong-kind
  marker lines without creating phantom fences; only a valid matching closer
  ends it.
- Scanner behavior covers two-marker non-openers, three-space versus four-space
  indentation, short closers, closers with trailing text, tab-terminated
  closers, backticks in backtick info, tilde info, and an unclosed fence
  extending through the doc block's end.
- Existing plain, `compile_fail`, and `text` fences retain their source keys,
  hidden/visible lines, doc-block identity, and companion-proof behavior.
- Duplicate run keys fail in either input order. Identical duplicates and mixed
  pass/ignored, pass/failed, and ignored/failed groups produce the registered
  duplicate plus aggregate outcome, with no later failure hidden by an earlier
  success. Duplicate groups absent from the scanned population and duplicate
  groups at `text` fences each produce one key-level `Orphan`.
- Status JSON round-trips numeric lines and `null`. The host-root violation
  renderer, the host renderer for Nix sentinel violations, and the Nix `jq`
  consumer all omit the colon-line segment when the line is absent and retain it
  when present.
- The host command graph shows exactly one auxiliary-workspace `--doc`
  invocation for `xtask` and one for `tools`. Their non-doc host commands retain
  lib/bin/integration coverage, and doctest-fences owns both captured doctest
  executions.
- Running the host doctest-fences step from outside the repository root scans
  the same roots, reads the same files, and addresses the same manifests as
  running it from the root.
- Focused scanner, reconciler, status/renderer, command-construction, and cwd
  regression tests pass; the normal implementation and ship gates pass.

## Boundaries

- No change to the root workspace's `cargo test --workspace --doc` policy,
  compile-fail companion rule, accepted fence vocabulary, scan-root population,
  or bidirectional reconciliation requirement.
- No attempt to support multi-line `#[doc = "…"]` reconciliation keys or source
  paths containing the run parser's literal `" - "` separator.
- No cwd contract change for the in-sandbox Nix/devtool producer.
- No unrelated xtask command-graph, doctest fixture, or documentation cleanup.

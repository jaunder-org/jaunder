# E2E Capture Recovery Outcomes

Approval: `e2e-capture-recovery-observation-v1` (Manager relayed human approval
`c63cdf94-0aa8-4ce4-820c-b96a08d6542c`: “Approve bounded diagnostics.”)

## Outcome

A failed SQLite/Chromium E2E build reports where its existing
`capture-{lane}.tar.gz` was sought and what happened while xtask recovered
diagnostics, so an unavailable capture is distinguishable from an unreadable
source, failed copy, or later destination change. The original Nix failure
remains the failed step and is never replaced by ancillary diagnostic status.

## Approach and tradeoffs

Add bounded, lane-qualified observations to the existing failed-output
diagnostic result. Cover both recovery routes: the evaluated Nix `outPath`, and
every matching `emit-out/diagnostics` directory. For each route/source, record
its identity, capture-file metadata state, copy attempt/outcome, and whether
authoritative-input cleanup ran. After all recovery routes and cleanup, record
the final destination metadata state. A successful metadata read that returns
NotFound may be recorded as absent; metadata, path traversal, or read errors are
explicitly unknown/error, never absence. Include a file size when available, not
trace contents or another copy of the large archive.

Use the existing `DiagnosticsCopy`/failed-build result path and test seams; keep
the observation in the existing step detail/last-result output. Do not add a
reporter, telemetry, output artifact, retry, or alternate recovery route.
Preserve the primary Nix status/detail and existing best-effort semantics.

## Acceptance

- A present capture copied from outPath is reported with source identity,
  outcome, and final destination presence/size.
- An absent capture and metadata/read errors are distinct (`absent` versus
  `unknown/error`); injected copy failure is identified by route and source.
- Multiple `emit-out/diagnostics` sources are individually represented and final
  destination state is evaluated after all recovery and
  stale-authoritative-input clearing; intermediate success is not reported as
  final success.
- Existing copy test
  `copy_e2e_diagnostics_between_copies_journal_capture_playwright_and_manifest`,
  injected failure test `copy_e2e_diagnostics_reports_a_file_it_could_not_copy`,
  and lookup test `failed_e2e_out_path_lookup_aggregates_with_build_diagnostics`
  cover the observable cases; add a focused test for multi-source recovery/final
  destination state. Tests verify the original failed Nix result survives every
  diagnostic outcome.
- Focused xtask unit tests pass. No browser/VM/e2e/CI run is part of this spec.

## Boundaries

Only xtask's existing failed-output and emitted-diagnostics recovery
observations and focused unit tests. No reporter/telemetry, recovery redesign,
tracker changes, collector tuning, test/assertion/population/timeout/gate
changes, push, PR, or merge work.

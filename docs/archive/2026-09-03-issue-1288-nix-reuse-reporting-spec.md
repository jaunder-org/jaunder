# Issue 1288: Report Nix derivation reuse

## Outcome

Successful Nix-backed xtask steps report which installable and derivation they
evaluated and whether the required outputs were already available locally or had
to be realized during the gate. Human output stays concise, while
`.xtask/last-result.json` exposes the same evidence as structured data for
repeatable invalidation measurements.

## Load-bearing decisions

- Nix evidence belongs to the host-side xtask result envelope established by
  ADR-0028; in-sandbox producers remain unchanged.
- Reporting covers every gate-owned `nix build` execution, including both the
  flake-check path and the separate `wasm-budget` build of `.#site`.
- A Nix-backed `StepResult` carries an optional structured `nix` object rather
  than encoding machine-readable fields in `detail`.
- The object identifies the requested installable and its evaluated `.drv` store
  path. The derivation path is the stable identity used to compare runs.
- Realization is a three-state result: `reused`, `realized`, or `unknown`.
- `reused` means every output selected by the installable was valid in the local
  Nix store before the build began.
- `realized` means at least one selected output was not valid before the build,
  and the successful build left every selected output valid.
- `unknown` means evaluation, metadata parsing, or local-store probes could not
  establish either state conservatively.
- The classification describes observable local-store state across the gate. It
  does not claim whether Nix built locally or substituted an output, and it does
  not parse Nix's internal event-log protocol.
- Metadata collection is observational and non-gating. A successful Nix build
  remains successful when metadata is unavailable or malformed; its realization
  is `unknown`, and unavailable identity fields are represented honestly rather
  than fabricated.
- Failed builds retain the existing primary stderr streaming, best-effort
  diagnostic capture, excerpt/rescue, and error-detail behavior. Metadata must
  not replace or suppress those diagnostics.
- Nix command and metadata execution are injectable at the boundary used by
  focused tests; tests do not depend on live local-store or remote-cache state.
- No new domain term or architectural boundary is introduced; this extends the
  existing xtask result-reporting surface and requires no ADR.

## Acceptance

- Each successful gate-owned `nix build` path, including flake checks and
  `wasm-budget`, serializes a structured Nix report containing its installable,
  derivation identity when available, and one of `reused`, `realized`, or
  `unknown`.
- Repeating a gate against already-valid outputs reports `reused` without
  requiring a remote cache.
- A successful gate whose outputs were absent at the initial probe and valid
  after completion reports `realized`.
- Failed, unavailable, or malformed evaluation and store-probe results produce
  `unknown` without failing an otherwise successful build.
- Human step output visibly names the realization state and derivation identity
  when available without printing raw Nix logs or full metadata documents.
- Existing failed-build diagnostics and failure precedence remain unchanged.
- Focused tests cover serialization, concise human rendering, all three
  classifications, incomplete selected-output sets, malformed metadata,
  unavailable commands, and successful-build/non-gating behavior through
  injected commands or pure observations.
- A catalog or integration assertion proves that every gate-owned `nix build`
  path attaches the structured report.
- Repository documentation provides a repeatable warm-baseline recipe for
  docs-only, web-only, and low-stack Rust changes, recording derivation
  identity, realization state, and duration from `.xtask/last-result.json`.
- Architecture documentation describing the Nix gate and result envelope remains
  accurate, including removal of stale source-line references encountered in the
  touched sections.

## Boundaries

- No changed-path routing; issue #1123 owns that policy.
- No Nix derivation splitting; issue #1289 consumes the measurements from this
  work.
- No attempt to distinguish local compilation from substitution or attribute
  realization to one process in the presence of concurrent Nix activity.
- No GitHub, Cachix-specific, or other remote-service dependency in
  classification.
- No weakening or reordering of validate, coverage, backend-parity, doctest, or
  e2e gates.

# Nix Derivation Reuse Reporting Implementation Outline

> Execute with `jaunder-iterate`, delegating through `jaunder-dispatch` when
> useful. This outline exists because issue #1288 extends the machine-readable
> `StepResult` protocol and two independent Nix-build paths must produce the
> same report.

## Scope

In:

- Structured per-step Nix identity and realization state.
- Deterministic metadata and local-store probes shared by flake-check and
  `wasm-budget` builds.
- Concise human rendering, focused tests, and the measurement recipe.

Out:

- Changed-path routing, derivation splitting, cache-source attribution, and Nix
  gate behavior changes.

## Task outline

- [x] Task 1: Define and render the Nix step report
  - Contract: `StepResult` gains optional structured Nix metadata containing the
    installable, optional `.drv` path, and a closed
    `reused | realized | unknown` realization value; non-Nix JSON remains
    unchanged.
  - Verification: serialization and human-rendering tests prove all states,
    optional identity, concise output, and no raw metadata/log dump.

- [x] Task 2: Observe selected Nix outputs without realizing them
  - Contract: the shared host-side Nix-build module parses
    `nix build --dry-run --json --no-link` stdout for the selected output paths
    and derivation, and probes those paths through offline
    `nix path-info --json --json-format 2`. Pure parsers and an injected command
    boundary distinguish all-valid, at-least-one-invalid, and indeterminate
    observations; no stderr prose or internal Nix event protocol is parsed.
  - Verification: focused fixtures cover multiple selected outputs,
    null/missing/malformed JSON, unavailable commands, probe failures, and the
    before/after realization classifier.

- [x] Task 3: Attach observations to every gate-owned Nix build
  - Contract: `steps::nix` flake checks and the separate `wasm-budget` `.#site`
    build take a pre-build observation, retain their existing build execution
    and failure behavior, and attach a finalized Nix report after success. Other
    callers of the shared output-path helper are migrated cleanly but do not
    claim gate evidence they do not expose as a `StepResult`.
  - Verification: focused integration seams prove successful
    reused/realized/unknown attachment, metadata failures remain non-gating,
    failed-build diagnostics and precedence remain unchanged, and a catalog
    assertion covers both gate-owned build paths.

- [x] Task 4: Document and verify invalidation measurements
  - Contract: the contributor gate/result documentation and architecture view
    describe the new fields and a warm-baseline docs-only, web-only, and
    low-stack Rust recipe; stale touched source-line references are corrected.
  - Verification: documentation formatting/link checks plus the xtask focused
    tests; implementation then follows the repository verify ladder and
    `jaunder-commit` gate.

## Risk checks

- Metadata collection must never realize an output before the measured build;
  dry-run and offline path probes are mandatory.
- The measured set is the outputs selected by the installable, not every output
  declared by its derivation.
- Missing or malformed metadata yields `unknown`; it never converts a successful
  build into a failed gate.
- Existing live stderr, diagnostic capture, rescue, exit/signal handling,
  installables, and out-link behavior remain unchanged.
- `wasm-budget` must report the `.#site` build without making explicit
  prebuilt-path audit commands pretend they invoked Nix.
- No second result schema or Nix command convention may be introduced beside the
  shared module.

# Validate Server-Function Coverage Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` when useful. This
> outline exists because the change crosses the public validate contract, Nix
> derivation-path ownership, and fail-closed result ordering.

## Scope

In:

- A reusable explicit-capture coverage verifier.
- An E2E-combination outcome returned by the aggregate runner.
- `validate` wiring that resolves the authoritative realized output without a
  second build.
- Focused stale/success/failure policy tests and accurate contributor guidance.

Out:

- E2E matrix, snapshot schema, flow-coverage, or authoritative-combo changes.
- Aggregate capture consumption or another VM realization.
- Informational flaky-report collection in `validate`.

## Task outline

- [x] Task 1: Verify server-function coverage from an explicit capture path.
  - Contract: `server_fn_coverage_check` accepts a caller-owned
    `capture-sqlite.tar.gz` path and reuses the existing fail-closed extraction,
    parsing, inventory, render, and byte-comparison logic. The standalone
    diagnostics-path entry point remains a thin caller of the same seam.
  - Verification: focused tests cover matching and stale snapshots plus missing,
    empty, malformed, and unreadable explicit captures without a VM.

- [x] Task 2: Expose whether the four E2E combinations produced trustworthy
      outputs.
  - Contract: `steps::nix::e2e` returns a named outcome derived from every step
    owned by the four catalog-ordered combinations, including post-build
    duration validation. It is independent of prior `CommandResult` failures and
    the trailing, unrelated elisp-integration result. The function still appends
    every existing result and preserves concurrent realization/order.
  - Verification: focused Nix-step tests prove all-pass, combo-build failure,
    post-build duration failure, prior global failure, elisp-only failure, and
    unchanged catalog ordering.

- [x] Task 3: Wire authoritative coverage verification into `validate`.
  - Contract: after `steps::nix::e2e`, a successful combo outcome resolves
    `.#checks.x86_64-linux.e2e-sqlite-chromium.outPath` through the existing
    `nix eval --raw` seam and verifies `<outPath>/capture-sqlite.tar.gz`. It
    invokes no `nix build`.
  - Contract: a failed combo outcome appends an explicit skipped
    `server-fn-coverage-verify`; a successful outcome makes resolution,
    filesystem, extraction, parse, and drift errors fail closed. Earlier
    unrelated validate failures do not suppress this step.
  - Verification: dependency-injected path-resolution tests prove success,
    stale-snapshot failure, resolver/capture failure, skip behavior, and that
    the resolver—not a builder—is the only post-aggregate command seam.
  - Contract: update `CONTRIBUTING.md` only as needed so the local-full-gate
    description names the restored coverage check and preserves CI matrix
    distinctions.

## Risk checks

- Do not inspect backend-collided aggregate paths.
- Do not derive combo success from global `CommandResult::ok`.
- Do not trigger or substitute a second individual E2E build.
- Preserve standalone `e2e sqlite chromium` coverage result names and ordering.
- Keep stale/missing evidence fail-closed after a successful aggregate.
- No lint suppression without explicit user approval.

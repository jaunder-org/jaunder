# Issue #856 WASM attribution implementation outline

> Execute with `jaunder-iterate`, delegating bounded work through
> `jaunder-dispatch`. This outline exists because the temporary build contrast,
> artifact identities, and conditional browser certification must remain ordered
> and auditable.

## Scope

In:

- Isolate and measure the marginal WASM cost of direct projector-seed decoding.
- Record a reviewable negative result or retain one qualifying JSON-compatible
  production reduction.
- Remove every temporary experiment artifact before delivery.

Out:

- `PageSeed` wire changes, ADR-0041 changes, `/pkg/*` caching, and #869.
- `server_fn` transport, browser telemetry, or WASM-budget redesign.

## Task outline

- [x] Task 1: Certify the baseline/removal byte contrast
  - Contract: one baseline revision; one exact temporary diff limited to the
    direct seed-decode boundary; otherwise identical Nix/toolchain inputs.
    Persist realized Nix outputs, artifact hashes, commands, total pre-bindgen
    code-section bytes, the `serde_json` row, shipped raw WASM bytes, and both
    arms' `serde_json` dependency paths.
  - Verification: the recorded baseline-minus-removal total code-section delta
    applies the inclusive `25 * 1,024`-byte threshold. Independently inspect the
    arm diff and artifact identities before accepting the contrast.

- [x] Task 2: Resolve the data gate
  - Contract: below threshold, write the negative verdict and proceed directly
    to cleanup. At or above threshold, attempt the smallest behavior-preserving,
    JSON-compatible candidate; it is eligible for browser capture only if
    shipped raw WASM is smaller. If no such candidate exists, record the
    attempted mechanism and measured result, then proceed to cleanup.
  - Verification: every no-candidate delivery contains the evidence for why the
    gate stopped. A retained candidate preserves the existing seed decoder,
    projector, route-matching, content-weight, escaping, cacheability, and
    first-paint contracts under their existing focused tests.

- [ ] Task 3: Certify any eligible candidate in browsers
  - Contract: conditional on Task 2 retaining a candidate. Before final capture,
    commit the final run count—at least three runs per arm, raised if the dry
    run requires it—and the spec's run-level noise-floor prediction to
    `docs/observability.md`. Capture SQLite, single-worker, quiescent,
    counterbalanced Chromium and Firefox arms with distinct salts and current
    `direct-init-v1` coverage, closure, and independent arm identity.
  - Verification: Firefox `wasmApiMs` improvement exceeds its noise floor in
    cold and warm populations; Chromium regresses beyond neither floor.
    `wasmInitMs` and exclusive boot total are reported but do not decide. Revert
    and record a candidate that misses the rule.

- [ ] Task 4: Publish the verdict and clean the branch
  - Contract: `docs/observability.md` contains the complete evidence chain and
    corpus location. The delivered tree contains no removal arm, temporary
    build/config flag, trace-only instrument, or other experiment scaffold.
  - Verification: inspect the branch diff for cleanup and scope, format changed
    documentation, run the focused changed-contract checks, then hand the exact
    staged tree to `jaunder-commit`.

## Risk checks

- Do not compare separately rebuilt artifacts without recording the revision,
  realized Nix output, and content hash for each arm.
- Do not use the `serde_json` attribution row or shipped raw-WASM delta as the
  25 KiB deciding value; the total attribution-artifact code-section delta
  decides.
- Do not infer boot improvement from byte movement. Browser certification is
  mandatory only for a retained production candidate.
- Do not let the removal arm or a below-noise candidate land as product
  behavior.
- Preserve ADR-0041 and ADR-0106; record surprising measurements rather than
  broadening the issue to chase bytes.

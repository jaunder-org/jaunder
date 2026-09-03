# Issue #856 — attribute `serde_json` WASM cost

## Outcome

Jaunder will determine how much of the CSR bundle's measured `serde_json` code
is marginally caused by projector `PageSeed` decoding. The result will either
justify a measured, behavior-preserving reduction or record that the direct seed
parse is not a material bundle-size lever.

## Load-bearing decisions

- Measure before changing the projector-to-CSR contract. The existing 145 KiB
  figure is a pre-wasm-bindgen code-section attribution, not evidence that the
  seed parser causes all of those bytes.
- Compare a baseline with a temporary removal arm that changes only the direct
  seed-decode path. The removal arm may skip seed decoding because it is an
  attribution instrument, not production behavior, and must not land on `main`.
- Build both arms from otherwise identical source, Nix inputs, optimization,
  wasm-bindgen, and `wasm-opt -Oz` settings.
- Report both the existing pre-bindgen code-section breakdown and the shipped
  optimized `pkg/jaunder.wasm` raw byte count. Raw shipped bytes remain the
  authoritative compiler-input proxy under ADR-0106; compressed bytes are
  supplementary.
- Record whether `serde_json` remains reachable through `server_fn`, browser
  telemetry, or other CSR dependencies. Retained transitive code is an expected
  measurement result, not a reason to broaden the experiment.
- Materiality is the baseline minus removal-arm delta in the attribution
  artifact's total code-section bytes, not the `serde_json` row or shipped raw
  WASM delta. Use binary units (`1 KiB = 1,024 bytes`); a delta of at least
  `25 * 1,024` bytes is material.
- For a material delta, investigate a behavior-preserving, JSON-compatible
  reduction first. It must reduce shipped raw WASM before browser capture.
- Firefox `wasmApiMs` is the deciding boot metric, evaluated separately for cold
  and warm populations from unpaired run-level arm means. For each
  browser/population, the noise floor is three times
  `sqrt(baseline_variance / baseline_runs + candidate_variance / candidate_runs)`.
  Keep a candidate only when its Firefox mean improvement exceeds that floor in
  both populations. Chromium is the control: its candidate mean may not regress
  beyond its own floor in either population. Report `wasmInitMs` and exclusive
  boot total as non-deciding diagnostics.
- Use a quiescent host, SQLite, single worker, distinct salts, counterbalanced
  arm order, current `direct-init-v1` coverage and closure, and at least three
  runs per arm. A dry run may set a larger final run count, but that count and
  the noise-floor rule must be committed before final capture.
- Preserve `PageSeed` variants, route matching, missing- and malformed-seed
  behavior, anonymous projector cacheability, HTML escaping, content-weight
  tiers, and flash-free first-paint coincidence.
- Publish the commands, baseline revision, exact removal diff, realized Nix
  outputs, artifact hashes, arm order and salts, byte results, dependency
  interpretation, corpus location, and verdict in `docs/observability.md`.

## Acceptance

- The baseline and removal arms differ only at the direct projector-seed decode
  boundary. The record identifies the baseline revision and includes the exact
  temporary diff plus realized artifact identities so isolation is auditable.
- Results include the attribution artifact's total code-section delta and
  `serde_json` row, shipped raw WASM bytes, and confirmation of the remaining
  `serde_json` dependency paths for both arms.
- The recorded verdict applies the inclusive `25 * 1,024`-byte threshold to the
  total code-section delta and does not infer boot-time savings from raw bytes.
- Every temporary source, build, attribution, or trace instrument is absent from
  the delivered branch; only a qualifying behavior-preserving production
  candidate may remain.
- If a production candidate is retained, existing seed decoder/projector tests
  pass unchanged in meaning and the documented Chromium/Firefox trace comparison
  satisfies the pre-registered run-level noise-floor rule.

## Boundaries

- No incompatible `PageSeed` encoding or change to ADR-0041.
- No `/pkg/*` naming, caching, or issue #869 work.
- No removal or redesign of `server_fn` JSON transport or browser telemetry.
- No new permanent experiment flag, alternate boot mode, or bundle-size budget.

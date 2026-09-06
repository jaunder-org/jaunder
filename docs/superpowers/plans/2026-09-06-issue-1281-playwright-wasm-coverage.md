# Playwright-driven Rust/WASM coverage implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independent
> slices. This outline exists because the Nix producer, browser export,
> Playwright capture, and host analyzer share a versioned artifact contract.

## Scope

In:

- A separately pinned diagnostic CSR build and matching LLVM tools.
- One canonical-config SQLite flow in each of Chromium and Firefox.
- Retained per-browser profiles, source reports, statuses, diagnostics, and a
  conditional count-union report.
- A host command that runs both producers, reconciles their evidence, and fails
  unless both establish original Rust-line coverage.
- Quiescent paired overhead measurements and durable findings.

Out:

- Changes to the stable toolchain, production CSR artifact, normal E2E matrix,
  coverage denominator, CI gates, or release optimization policy.
- JavaScript coverage percentages or a `wasm-bindgen-test` substitute.
- A permanent Rust/WASM coverage gate.

## Task outline

- [x] Task 1: Produce an instrumented, source-mappable diagnostic CSR bundle.
  - Contract: `client::diagnostic_coverage` owns diagnostic-only browser glue
    around the pinned `minicov` dump and module signature;
    `common::diagnostic_coverage` owns target-independent capture classification
    and coverage-section policy. The Nix-only manifest/runtime overlay is
    source-verified and does not enter the production workspace lock, vendor
    inputs, or CSR closure. A separate Nix toolchain/CSR derivative retains the
    exact source-mappable module, served-module and retained-module SHA-256
    digests, source identity, compilation-directory path equivalence, matching
    LLVM tools, exact `csr` instrumentation, and the named omitted first-party
    crates. Production `csrWasm` and `csrWasmBundle` inputs remain byte-for-byte
    unchanged.
  - Verification surface: focused `common` contract and `devtool` parser tests
    cover these public seams; the diagnostic Nix output records its
    Rust/LLVM/minicov identities and explicitly reports whether
    production-equivalent `wasm-bindgen`/`wasm-opt` processing preserved the
    required coverage metadata.

- [x] Task 2: Capture independent Chromium and Firefox profiles from one real
      CSR flow.
  - Contract: add one focused diagnostic Playwright spec loaded by
    `end2end/playwright.config.ts`; its driver supplies SQLite, browser,
    one-worker, no-retry, diagnostic bundle, and writable artifact inputs. Each
    Nix producer always retains diagnostics and writes one `v1` browser status
    containing browser identity; structural, export, and mapping outcomes; an
    exact blocker for each failed outcome; module signature and toolchain
    identity when export starts; and an exhaustive manifest of every retained
    artifact. References are relative and conditional: the exact module and
    diagnostics are always present, while raw-profile and mapped-report
    references are required only after their producing stages pass and forbidden
    when those stages did not run. Unsupported capture is evidence, not an
    omitted status or synthetic success.
  - Verification: execute both browser producers. Each runs the real CSR page
    with one document load and either yields a nonempty raw profile plus
    original Rust-line report or a structurally valid explicit blocker; both
    outputs are retained regardless of the other browser's result. A
    deterministic injected export/mapping failure proves each producer still
    writes its status, exhaustive diagnostic manifest, and exact blocker before
    returning control.

- [ ] Task 3: Reconcile browser evidence and prove conditional profile union.
  - Contract: a host `cargo xtask wasm-coverage probe` command realizes both Nix
    producers before deriving its verdict. Its versioned aggregate payload has
    exactly `chromium` and `firefox` entries copied from validated producer
    statuses, artifact roots, and a verdict derived solely from their
    structural, export, and mapping outcomes. Matching LLVM tooling performs
    count-summing union only when both entries pass; malformed, stale,
    mismatched, partial, or missing evidence fails closed without erasing either
    producer's artifacts.
  - Verification: focused xtask tests cover status parsing, exact browser
    population, exhaustive manifest-to-disk reconciliation, artifact-path
    containment, outcome-dependent required and forbidden references,
    contradictory evidence, failure accumulation, derived verdict, and merge
    eligibility. An executable probe run retains both result sets and exits zero
    only if both map executed original Rust lines and the merged report shows
    their union.

- [ ] Task 4: Measure overhead on a quiescent host and publish the viability
      finding.
  - Contract: after Tasks 1–3 establish functional viability, coordinate the
    quiescent window with the user. Alternate five warmed same-nightly baseline
    and instrumented runs per browser. Every member has a distinct retained
    cache-buster proving fresh Nix execution. Record median and range for the
    focused flow duration and raw uncompressed `pkg/jaunder.wasm` bytes. If
    functional viability fails, skip timing and state why.
  - Verification: the retained measurement manifest reconciles ten fresh runs
    per browser when timing occurs. `docs/coverage/playwright-wasm-coverage.md`
    separately records primary and executable evidence for the pinned toolchain,
    profile and retained-artifact formats, Chromium and Firefox behavior,
    optimizer effects and deviations, source reconciliation, count-union
    semantics, measured or skipped runtime overhead, Nix/host sandbox ownership,
    exact blockers, and the final viability verdict for a permanent both-browser
    fail-closed gate.

## Key contracts

- Browser status schema `v1`: browser, structural/export/mapping outcomes,
  blocker details, conditional module signature and toolchain identity, plus an
  exhaustive contained-relative-path manifest whose required and forbidden
  entries follow the producing stage outcomes.
- Aggregate schema `v1`: exact Chromium/Firefox population, validated artifact
  roots and exhaustive manifests, optional merged-report reference, and a
  mechanically derived verdict.
- Producer behavior: preserve artifacts and report failure data; consumer
  behavior: validate complete evidence and own the command exit status.
- Source identity: reports use the exact served instrumented module and source
  revision, with explicit compilation-directory or path-equivalence mapping.

## Risk checks

- The diagnostic feature/dependency cannot enter host/server builds or the
  production CSR closure.
- The canonical Playwright config remains the only config; diagnostic selection
  is invocation-owned and preserves the one-boot rule.
- LLVM raw profile, mapping format, runtime, `llvm-profdata`, and `llvm-cov`
  versions remain compatible and recorded together.
- Both browser producers run even after one fails; unsupported behavior cannot
  become absent or green evidence.
- Optimizer or bundler deviations are measured and disclosed rather than hidden.
- Cache-busting is runtime-owned, retained as evidence, and never committed as a
  nonempty repository salt.
- Relevant command, coverage, and experiment documentation is updated; no ADR or
  `CONTEXT.md` change is warranted unless implementation discovers a durable
  architectural or domain decision not captured by existing ADRs.

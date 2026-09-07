# Playwright-driven Rust/WASM coverage experiment

## Outcome

Jaunder gains a reproducible diagnostic probe that determines whether the real
CSR application can export LLVM profiles after Playwright execution and map
those profiles to original Rust source lines in both Chromium and Firefox. The
probe preserves per-browser evidence and a durable findings document; it does
not add a permanent coverage gate.

## Load-bearing decisions

- The normal Rust 1.97.1 toolchain, production CSR derivation, optimized
  production artifact, and existing SQLite/PostgreSQL × Chromium/Firefox E2E
  matrix remain unchanged.
- The diagnostic CSR is a separate Nix-owned derivative using an independently
  pinned nightly Rust toolchain and matching LLVM profiling tools. Raw profiles
  are processed only by tools compatible with the compiler that produced them.
- The probe adapts the upstream `minicov` dump and module-signature mechanism
  into a diagnostic-only application export. Browser-facing export glue belongs
  in `client`, with target-independent profile/signature logic kept
  host-testable outside the wasm-only implementation. It does not run the
  application as a `wasm-bindgen-test` or substitute a test-runner page for
  Jaunder's CSR page.
- The diagnostic bundle starts from production bundling and optimization
  settings. Every deviation required to retain instrumentation, profile runtime,
  or `__llvm_covfun` / `__llvm_covmap` metadata is explicit and recorded as
  viability evidence.
- Nix owns the instrumented build, pinned browsers, writable profile capture,
  and per-browser artifacts. Host `xtask` invokes the producers, validates their
  structural status, analyzes source mapping, and presents the result, following
  ADR-0028's producer/consumer boundary.
- The execution population is SQLite × Chromium and SQLite × Firefox. Each runs
  the same focused real CSR flow through the canonical
  `end2end/playwright.config.ts`, with one worker, no retries, and the existing
  one-document-load discipline. Diagnostic differences are invocation inputs,
  never a second config or a runner-specific branch inside the config. Database
  parity remains the responsibility of the unchanged E2E matrix.
- Chromium and Firefox produce independent raw profiles, module signatures,
  mapping reports, diagnostics, and statuses. A versioned aggregate status has
  exactly one entry for each browser, recording structural, export, and mapping
  outcomes, an exact blocker when any outcome fails, and references to every
  retained artifact. Its overall verdict is derived from those entries. The
  aggregate always runs both and retains both result sets before failing unless
  both browsers map executed counters to original Rust lines.
- Profiles are merged only after both browser results pass structural and source
  reconciliation. LLVM's normal count-summing union is used, while independent
  reports preserve browser attribution.
- Source reconciliation uses the exact instrumented module and source revision,
  with explicit compilation-directory or path-equivalence mapping where the Nix
  build path differs from the checkout. Generated JavaScript coverage is never
  reported as Rust/WASM line coverage.
- Runtime overhead is measured only after functional viability is established.
  During a user-coordinated quiescent-host window, the probe alternates five
  warmed, paired same-nightly baseline and instrumented runs per browser. A
  distinct retained cache-buster proves every measured derivation executed
  afresh. The findings report median and range for flow duration and the raw
  uncompressed `pkg/jaunder.wasm` byte size for each build. These observations
  have no pass/fail threshold; they inform the viability finding.
- The findings cite primary or executable evidence for the pinned toolchain,
  profile and artifact formats, browser behavior, optimizer effects, source
  reconciliation, merge semantics, runtime overhead, sandbox ownership, and
  whether a future fail-closed gate is viable.

## Acceptance

- One command reproducibly builds and runs the diagnostic probe and emits a
  versioned machine-readable aggregate status whose two browser entries,
  outcomes, blockers, artifact references, and derived overall verdict reconcile
  exactly with retained per-browser artifacts.
- A real Jaunder CSR flow executes under both pinned Chromium and pinned
  Firefox, after which each browser either exports a nonempty profile with its
  module signature or records an explicit unsupported/failure reason.
- For every successful browser result, matching LLVM tools merge its raw profile
  and `llvm-cov` reports executed original Rust source lines from the exact
  instrumented module; source-path reconciliation is recorded and repeatable.
- When both browser results succeed, a merged report demonstrates count union
  without erasing the two independent reports. Missing, stale, malformed,
  mismatched, or partial evidence makes the aggregate command fail.
- The quiescent five-pair protocol retains a distinct execution cache-buster for
  every measured run and records comparable baseline and instrumented flow
  timings plus raw uncompressed `pkg/jaunder.wasm` byte sizes for both browsers.
- A committed findings document states viable or not viable for a permanent
  both-browser fail-closed gate and supports that conclusion with retained
  executable evidence and primary-source citations.
- If either browser or original-line mapping is unsupported, the probe still
  completes evidence collection for both browsers, exits nonzero, names the
  exact blocker, and adds no JavaScript-glue percentage as a substitute.

## Boundaries

- No permanent coverage threshold, denominator change, exemption mechanism, or
  CI gate is introduced by this issue.
- No production CSR, normal E2E project population, backend coverage policy, or
  release optimization setting changes to accommodate the experiment.
- No dedicated `wasm-bindgen-test` flow stands in for Playwright exercising the
  real application.
- No timing conclusion is drawn from the non-quiescent host; quiescent execution
  is coordinated only if the profile export and source mapping path succeeds.

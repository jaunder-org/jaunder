# Issue #1981 — reuse dependency artifacts in the Rust coverage producer

## Outcome

`checks.x86_64-linux.coverage` (and therefore `coverage-gate`) stops recompiling
every dependency on each run. The producer's `status.json`,
`coverage-report.txt` and `crap-report.json` keep their current contract.

## What actually happens today

CI run 44383337174 (issue comment, 2026-11-18) spent 724 s in the build phase:
the test census compiled 654 crates (315 s), and the instrumented `jaunder`
executable build then rebuilt 178 registry and 23 workspace units (102 s).

The issue guessed that instrumentation changes `RUSTFLAGS` for dependencies. It
does not. cargo-llvm-cov 0.9 instruments through `RUSTC_WRAPPER`, which adds
`-Cinstrument-coverage --cfg=coverage` only to the crates named in
`__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES`, the workspace crates. A local
probe confirmed that dev-profile dependency artifacts built without the wrapper
stay fresh when the same build reruns under `show-env`.

The two real causes:

1. **Wrong profile.** The derivation inherits `cargoArtifacts`
   (`buildDepsOnly hostArgs`). Crane builds that in its default `release`
   profile, with LTO and `opt-level = 'z'`. nextest and `cargo build` use the
   `dev`/`test` profiles, so none of the 1.1 GiB that CI fetches and unpacks (66
   s) is usable. `checks.doctests` has the same defect: its
   `buildDepsOnly (hostArgs // leanDevAndTestProfile)` sets debug levels for
   `dev`/`test` but still builds `release`.
2. **A second feature set.** `cargo build --package jaunder --bin jaunder`
   resolves features for one package without dev-dependencies. The workspace
   test build resolves them for every package with dev-dependencies, so the
   shared dependencies get different fingerprints and compile a second time.

## Approach

- **One shared test-profile dependency set.** In `nix/packages.nix`, define one
  `buildDepsOnly` (pname `jaunder-test`) over
  `hostArgs // leanDevAndTestProfile` with `CARGO_PROFILE = ""`. An empty value
  makes crane pass no `--profile`, so Cargo uses its own `dev`/`test` profiles,
  the ones nextest and `cargo test --doc` use. It runs only the
  `cargo test --locked --no-run` pass. Crane's default `check --all-targets` and
  plain `build` passes would add units that neither consumer uses, because they
  resolve different feature sets. `checks.coverage` and `checks.doctests` both
  use the set in place of their current artifacts. Both also take their debug
  settings from `leanDevAndTestProfile`, so the values that must match
  fingerprints come from one place. The name has no `coverage` in it, so the
  Cachix `pushFilter` (`jaunder-coverage|jaunder-e2e`) still pushes it and later
  runs fetch it instead of building it. The distinct name also makes its build
  and fetch easy to find in CI logs.
- **Reuse the executable the test build already makes.** The workspace test
  build already compiles the instrumented `jaunder` binary, because `server`'s
  integration tests need `CARGO_BIN_EXE_jaunder`. The census output the producer
  already captures (`nextest list --message-format json`) lists it under
  `rust-build-meta.non-test-binaries` as kind `bin-exe`, with a path relative to
  `target-directory`. The producer reads `JAUNDER_TEST_EXECUTABLE` from there
  and drops its separate `cargo build --package jaunder --bin jaunder`. As a
  result, `JAUNDER_TEST_EXECUTABLE` and `CARGO_BIN_EXE_jaunder` now name the
  same binary, by design. That binary is built with workspace-unified test
  features, and CONTRIBUTING's "corresponding instrumented executable" wording
  still holds.
- **No split, no crane coverage helpers.** `craneLib.cargoLlvmCov` runs
  `cargo llvm-cov test`, without nextest, the census, JUnit reconciliation or
  the controlled-red `status.json`. `craneLib.cargoNextest` covers only the test
  run, and the producer's stage accounting would have to wrap it again. Moving
  reports and CRAP into downstream derivations would mean handing over the
  instrumented binaries (a 12 GiB target directory) or a merged profile plus
  every object file. That saves at most the 38 s report stage when only the
  report stage changes. Neither change simplifies the system or cuts time.

Accepted cost: the shared dependency set is a new cached derivation, larger than
the release set because it includes test-only dependencies at debug level 0. Its
first CI build costs one cold dependency compile. Later runs fetch it.

## Acceptance

- The producer captures cargo output in `diagnostics/nextest-list.json` (census)
  and `diagnostics/nextest.log`; the Nix log does not show it. In the uploaded
  `validate-coverage-diagnostics` artifact, neither file shows a `Compiling`
  line for a registry crate. Only workspace crates compile, under the wrapper.
- The PR reports the coverage span before (run 44383337174, 819.9 s) and after,
  from a PR CI run whose shared dependency set was fetched from the cache. It
  also reports the dependency set's cold build time and fetch size.
- `coverage-gate` passes, and host `coverage-check` accepts the reports. The
  `status.json` population matches the earlier run's count, adjusted for tests
  added since then.
- `doctests-gate` passes on the shared dependency set.
- Producer unit tests pin census-based discovery: exactly one `jaunder`
  `bin-exe` entry, resolved to an absolute path, and an error for a missing or
  duplicate entry.

## Boundaries

- Do not change the coverage population, the report formats, the nextest
  profiles or the `status.json` schema.
- Do not add per-stage child spans or `cargo --timings` diagnostics (the
  instrumentation gap in the issue comment). File a follow-up if no existing
  issue covers it.
- Do not change other derivations' dependency artifacts beyond `doctests`.

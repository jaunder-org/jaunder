# #1804: Reuse the Nix CSR bundle for focused Rust tests

## Outcome

Repeated `cargo xtask test-local` invocations reuse the complete, source-correct
CSR bundle instead of regenerating it before every Rust test. Its consumers,
including `check` and `prepush`, inherit the improvement without reducing their
test populations or accepting a cached test verdict.

## Approach and tradeoffs

When `JAUNDER_CSR_BUNDLE_DIR` is absent, resolve the current checkout's existing
Nix `.#csrBundle` package for the native platform. Nix owns source/configuration
identity, realization and local reuse; existing CI publication and the
configured Cachix substituter provide shared artifact reuse. Consume the
immutable output directly, not `.#site`, which omits the build-only manifest and
system artifacts. The producer must come from the evaluated checkout, not an
ambient PATH devtool. No second fingerprint, artifact registry or cache-validity
protocol is added.

The bundle is release-built. Host Rust tests and the real application image
worker retain their current debug profiles. `build-csr`, sandbox and default
`e2e-local` retain their existing debug workflows. Release CSR omits
browser-side debug assertions; a cache miss may cost more than a native debug
build. These tradeoffs are accepted for the Rust-test prerequisite, not for all
development. A separate debug Nix variant would duplicate an artifact already
produced by CI; we do not need it to preserve the explicit browser-debugging
paths.

Before selecting an implicit bundle, reject relevant nonignored untracked inputs
that the Git-backed flake would omit. Determine relevance from Nix's declared
producer/source inputs, reusing the existing tooling-admission approach rather
than maintaining a parallel source list. Tell the caller to use `git add` or
`git add -N`; never stage automatically. Tracked dirty edits and intent-to-add
inputs must participate. Unrelated untracked notes must remain harmless.

Keep the existing complete-bundle validation: manifest, representations,
rendered shell and system-artifact inventory. An explicitly supplied bundle
remains an intentional caller-owned input with the existing source-equivalence
and stable ownership requirements; an invalid input fails without fallback
production. Do not turn its integrity hashes into an unsupported provenance
claim.

Use an ordinary checkout-local Nix GC root to retain the selected immutable
output through consumer compilation/execution. Preserve the existing host
artifact lock so cooperating local consumers cannot replace their active root.
Do not copy the output over `target/site` or add a cleanup/admission framework.
A failed admission, realization or validation prevents consumer execution; never
fall back to an old bundle. Package realization is not a hermetic test/check
verdict: the existing host image-worker build and nextest command still run.

## Acceptance

- An unchanged focused Rust command selects the same complete bundle without
  rerunning CSR production; demonstrate materially cheaper repeated feedback
  against #1751's frozen baseline using ordinary command timings.
- Nix identity changes for relevant CSR/shared source, producer, shell,
  application CSS/theme inputs, toolchain, profile and native dependency
  changes. Preserve required inputs; do not narrow existing broad closures to
  manufacture hits.
- Regression proof covers relevant untracked-input refusal, intent-to-add and
  tracked dirty inputs, harmless unrelated notes, and malformed or unavailable
  explicit/implicit bundles. Refusals occur before the Rust consumers run.
- The chosen output remains rooted and stable through the consumer lifetime;
  retain concurrent-call protection without running destructive GC experiments.
- Exercise the real focused SQLite/PostgreSQL Rust path and a startup-capable
  HTTP integration path using the selected bundle, not just synthetic manifests.
  Normal hooks and applicable CI remain required; browser/backend and coverage
  policies are unchanged. No cached CI verdict substitutes for fresh checks.
- Keep native-platform package selection compatible with Linux and Apple Silicon
  macOS; do not hardcode Linux. Linux proof does not claim native macOS
  execution.
- Commit regression tests and updated contributor guidance. Timing logs and
  source-identity observations stay in ignored run storage or PR evidence.

## Boundaries

No new command or profile flags, local-verdict cache, CI topology change,
browser setup refactor, cache purge, or broader build-system replacement. #1752
owns compiled-dependency reuse and broader source-closure improvements. Existing
Cachix publication is reused; no local upload credentials are required. Do not
expand this issue into the paused per-unit caching experiment or a new baseline
measurement campaign.

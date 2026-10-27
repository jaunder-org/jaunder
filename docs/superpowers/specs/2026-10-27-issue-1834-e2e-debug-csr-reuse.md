# Issue #1834: Source-correct debug CSR reuse for local E2E

## Outcome

Ordinary `cargo xtask e2e-local` reuses a complete, current DEBUG CSR bundle
instead of regenerating its packaging on every invocation. Browser behavior,
fresh server/database/capture lifecycles and existing test authorities remain
unchanged. This implements
[#1834](https://github.com/jaunder-org/jaunder/issues/1834), following the
completed
[#1833 investigation](https://github.com/jaunder-org/jaunder/issues/1833#issuecomment-6450697513).

## Approach and tradeoffs

Extend the existing Nix CSR WASM producer with a dev-profile variant and feed
that artifact to the same bundle postprocessor used today. Export the complete
native bundle as `.#csrDebugBundle`; keep `.#csrBundle` and existing release
outputs unchanged. Use pinned Crane's `CARGO_PROFILE=dev` for both dependency
preparation and the final WASM build, retaining the WASM target, default
features and pinned native tool inputs. The copied artifact comes from Cargo's
`debug` directory. Debug assertions remain enabled; this is not release
substitution.

The automatic path uses the repository's pinned toolchain and declared Cargo
configuration, not arbitrary ambient host toolchains, flags or profile
overrides. The shared postprocessor retains its existing wasm-bindgen, wasm-opt,
compression, shell and system-artifact behavior. A separate debug artifact costs
retained store space and may make the first miss slower; the benefit is
unchanged-input reuse. Nix already owns source/tool identity and artifact reuse.
A local freshness cache would duplicate that authority, while the existing
release bundle changes debug semantics and the output manifest alone cannot
prove producer freshness.

Reuse the existing Nix source-admission predicate and complete-bundle validator.
Tracked dirty and intent-to-add producer inputs participate in the Git flake;
relevant nonignored untracked inputs are refused with instructions to include
them explicitly. No automatic staging, second source inventory or fingerprint
protocol. Identity includes the relevant Cargo sources/manifests/locks/build
scripts/configuration, producer sources/locks, toolchain/native tools, HTML
shell, CSS, closed system-theme inventory and Nix configuration.

Without an explicit bundle input, ordinary E2E realizes and validates the debug
output at a distinct checkout-local `.xtask/csr-debug-bundle` Nix GC root. It
passes the resolved absolute immutable bundle through `JAUNDER_CSR_BUNDLE_DIR`
when compiling the host server; setting it only when spawning would be too late
because the server embeds assets at build time. Do not copy or link this
artifact over `target/site`. Host server and test-support binaries remain
debug-built.

An explicitly set `JAUNDER_CSR_BUNDLE_DIR` remains intentional caller-owned
input: resolve and validate it before server compilation, without automatic
production or claiming its profile/provenance. Its owner must keep it stable and
retained through consumption; our checkout lock cannot protect another owner's
artifact. Invalid explicit input is an error, never permission to choose an
automatic root. This automatic/explicit selection applies only to ordinary E2E.
Visual updates continue generating release CSR through their existing host path,
with their current inherited server-build override behavior; no new override
policy applies to visual mode.

The prepared host-artifact value owns the existing checkout-local lock guard
acquired before bundle selection, and E2E orchestration retains that value
through validation, consumer compilation, every browser lifecycle and teardown.
Nested preparation uses that held ownership rather than acquiring a second lock.
Early errors and interruption release ownership after owned consumers are torn
down; there is no unlocked handoff. The automatic root remains available for
warm reuse after completion under existing cleanup policy. Admission,
realization, root resolution or validation failure stops before consumer
compilation/startup, with no previous-root, release or `target/site` fallback.
Reuse artifacts only: every invocation executes fresh tests.

## Acceptance

- Evaluate the new native debug output and preserve existing release output
  definitions. Establish dev-profile/debug-assertion, target, feature, toolchain
  and shared-postprocessor behavior from the resulting producer configuration.
  Evaluate supported native-system wiring; report native execution only where
  exercised, without representing Linux proof as Darwin execution.
- Prove unchanged input selects the same debug derivation/output and avoids
  packaging realization. Probe an existing CSR Rust source, `csr/Cargo.toml`,
  root `Cargo.lock`, `tools/devtool/src/csr_bundle.rs`, `tools/Cargo.lock`,
  `csr/index.html`, `server/assets/jaunder.css`, an existing system-theme
  source, `rust-toolchain.toml` and `.cargo/config.toml`: each relevant edit
  must change the final debug bundle derivation identity. An unrelated note must
  not. Admission is separate proof: relevant new source is refused while
  untracked and included after intent-to-add; a tracked dirty edit also changes
  identity.
- Exercise invalid/missing bundles, failed realization despite an older root,
  explicit-input handling and competing preparation. Failures prevent consumer
  use; held ownership prevents another cooperating invocation replacing the root
  or host binaries until teardown. Cover error/interruption release without
  changing shared-cache ownership or introducing a scheduler.
- Run the same real `posts.spec.ts:139` scenario and unchanged warm repeat with
  SQLite, Chromium and one worker. Observe its rendering assertions, fresh
  lifecycle and zero-panic checks passing. Compare ordinary stage timings with
  #1833's successful observations; demonstrate packaging reuse, not cached test
  results. Keep measurements transient and do not recollect #1751's baseline.
- Preserve visual-update release mode, browser selection, standalone
  `build-csr`, sandbox and `test-local` behavior. Add focused behavioral
  regressions for invalidation, admission and artifact ownership where
  uncertainty warrants them; update CONTRIBUTING and the architecture
  description. Existing hooks and authoritative backend/browser CI remain
  unchanged.

## Boundaries

No new CLI/profile switch, release fallback, automatic cleanup, generic caching
framework, remote-cache rollout or build-system replacement. No changed browser
UI, test populations, assertions, timeouts/retries, coverage or gate policy. No
resumption of #1753/#1754 or expansion into #1709. ADR-0175's output-manifest
ownership remains unchanged; this extends the existing producer/reuse pattern.

Implementation starts only after this spec and the short concurrency/lifetime
outline receive their required approvals. Merge/deployment approval is separate.

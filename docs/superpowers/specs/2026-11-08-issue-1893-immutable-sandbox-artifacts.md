# Immutable sandbox and e2e build artifacts — #1893 spec amendment

Issue: [#1893](https://github.com/jaunder-org/jaunder/issues/1893)

Amends the previously approved S1/O1 contract; it does not replace the current
architectural record at `docs/adr/drafts/standalone-ux-sandbox-lifecycle.md`.

## Outcome

The sandbox and `e2e-local` consume one source-consistent set of immutable
Nix-built server, seed-helper, and CSR artifacts instead of publishing or
selecting those application outputs through shared mutable checkout build paths.
Nix owns build/store concurrency and reuse; consumers hold the selected outputs
rooted for their full use. Existing user-visible lifecycle and profile behavior
remains unchanged.

## Approved direction and provenance

The human approved the immutable-artifact direction in response to the parent
proposal to replace shared mutable artifact publication with immutable Nix
outputs for server, seed helper, and CSR, pass exact paths to sandbox/e2e, stop
`target/site` publication in those flows, let Nix own build locking/cache, and
remove the resulting artifact lock. The parent warned that this changes the
current Cargo-debug artifact workflow and needs a qualified amendment. Human
answer, preserved exactly: **“That is the path we should go, then.”** This
approves direction, not this amended specification or implementation.

After source findings raised release-versus-debug selection, the human said:
**“I think we probably want to retain the debug builds, just because those often
go faster.”** Preserve debug host-server and seed-helper defaults and the
current e2e debug/release CSR selection. Do not silently standardize on release
or retire the selection. Prefer the existing Nix builders and `csrDebugBundle`,
with narrowly scoped debug variants and one narrow immutable executable-bundle
composition if required. Exact package naming belongs in the approved outline.
Consumers receive exact server and seed executable paths from one
source-consistent selected artifact set. The Nix-built Jaunder executable
already embeds its selected CSR; no runtime CSR path is introduced.

The existing process direction remains governed by the human's exact words:

> “So, I think we should first look at slimming down the rust code by writing
> simple, straightforward code using processkit. My suspicion is that the code
> you actually need to write is so simple, and so much of it is really just
> calls to processkit, that your opportunties for testing might actually be kind
> of minimal.”
>
> “If you can produce a failing test, we can file a bug. You're free to add
> whatever code you need to be able to recognize and diagnose the failure in
> production, but otherwise trust that it works the way it says it works.”

## Approach and tradeoffs

### Immutable artifact flow

- Reuse existing Nix package/build definitions for Jaunder, `test-support`, and
  CSR. Jaunder’s Nix build stages its selected CSR bundle into the executable;
  do not add a runtime CSR asset interface or custom manifest/copying layer.
- Build narrowly scoped debug host and seed outputs and compose/select the
  matching CSR choice through existing builders. Preserve the e2e CSR selection
  behavior: debug by default and the existing release-CSR selection when
  requested. Keep exact paths and source/profile identity together so a consumer
  cannot combine artifacts from different selections.
- Sandbox and `e2e-local` consume those immutable Nix outputs instead of
  Cargo/local Vite publication of the application artifact set. Retain the seed
  subprocess boundary (`test-support seed-sandbox-profile`). Nix owns store
  build locking and immutable reuse; consumers retain ordinary Nix GC roots
  using existing rooted-selection practice for the entire server/test consumer
  lifetime.
- Remove `.xtask/host-artifacts.lock` acquisition or retention only from the
  sandbox and e2e artifact flows that no longer publish/read shared mutable
  application outputs. Keep the shared lock mechanism and `test-local`'s
  separate use. Keep direct local `build-csr` and its Vite `target/site` path
  unchanged for manual local use.
- Preserve active-checkout source-selection/admission policy: tracked changes to
  relevant package inputs must affect the selected artifact; relevant untracked
  inputs must be admitted through the existing supported mechanism or explicitly
  rejected, never silently omitted in favor of stale artifacts. Do not add a
  cache framework, ignore/pin/trust/offline/resource setting, or claim upload,
  substitution, or performance improvement from cache configuration alone.

### Process ownership and simplification

ProcessKit supplies child/group mechanisms. Application policy remains in
`ux-sandbox`: signals and returned status, readiness, metadata,
workspace/storage admission, locks, reset/recovery, and lifecycle ordering. A
source audit found roughly 110 teardown lines of manual membership polling/retry
overlapping the library's stop/report, plus a six-operation wrapper-recovery
fault matrix and scaffolding. This is a meaningful simplification target, not
evidence that all tests are ProcessKit tests or that deleting exactly 110 lines
is guaranteed. A higher-level API has not been shown to combine all application
policy. Keep only the justified application integration for the supported
stop/report operation, concurrent direct-child wait/reap, and exact signal
driving; retain a small set of meaningful application integration tests. Trust
documented COTS behavior rather than duplicating its internals, but preserve
actual application ordering and fail-closed behavior. An instantaneous `R`
process state is not by itself an established ProcessKit defect and does not
authorize a new library fault matrix.

The immutable artifact direction trades shared mutable checkout outputs and
artifact-lock contention for Nix derivation/package wiring and selected store
roots. Debug outputs may retain faster developer iteration but require
corresponding build variants. The change is successful only if it removes real
redundant publication/lock/polling paths and reduces total maintained machinery,
not if code is merely moved. No performance or cache-hit guarantee is made.

## Acceptance

1. **Artifact identity and lifetime:** Sandbox and e2e receive exact server and
   seed executable paths from one source-consistent artifact selection; CSR is
   embedded in the selected Jaunder executable. Existing Nix outputs are rooted
   for the full consumer lifetime through established root-selection practice.
   Relevant tracked checkout edits affect selected outputs; relevant untracked
   inputs are admitted or explicitly refused.
2. **Profile behavior:** Debug server and seed outputs remain defaults. E2e
   retains its current debug CSR default and release-CSR selection behavior. No
   release standardization or flag retirement occurs without a new approval.
3. **Lock/publication removal:** Migrated sandbox/e2e paths no longer publish or
   consume shared mutable `target` application outputs and no longer
   acquire/retain `.xtask/host-artifacts.lock` for them. `test-local`'s distinct
   lock use and the shared lock implementation remain. Manual
   `just build-csr`/Vite `target/site` behavior remains unchanged. Nix owns
   store build serialization; do not claim all locks are removed.
4. **Lifecycle compatibility:** Preserve the approved S1/O1 contract: disposable
   and named workspaces; profiles, init, resume, reset and recovery; seed
   subprocess; argv and child status; metadata/schema and executable
   digest/generation fencing; workspace/runtime/server lock ordering; readiness;
   and signal behavior (10-second automatic grace, further SIGINT may force
   earlier, repeated SIGTERM does not escalate). Nix daemon/build infrastructure
   is not an owned child to terminate. No guarantee is added for hostile process
   escape, helper SIGKILL, or every OS condition.
5. **Simplification and proof:** Demonstrate removed production
   publication/lock/polling paths and a narrower meaningful application test
   surface; do not count relocation or coverage manipulation as simplification.
   Keep the two existing workspace JSON decode-error tests. After implementation
   review and clean intended-tree commit, run fresh authoritative coverage and
   Nix proof plus focused/native proof appropriate to the change. Prior
   certification remains RED; do not infer green or reuse old-tree proof. Follow
   existing Linux production and periodic committed macOS cadence; no new
   foreign-native matrix.

## Boundaries

- No change to `ux-sandbox`'s Just/Cargo helper bootstrap, optional helper
  package exposure, cache-publication workflow, Python rewrite, or language
  decision. Those remain separate holds. This app-artifact approval does not
  authorize the cache-bootstrap A2 proposal or claim actual Cachix
  upload/substitution.
- No changes to `server/src/build_staging.rs`, `server/tests/build_script.rs`,
  immutable output permissions, `target/site` local manual flow, or #1923's
  owner-write repair. #1923 owns that small local-copy repair; keep store
  outputs read-only and preserve fresh-local-copy semantics.
- No xtask orchestration dependency in `ux-sandbox`, new generic shared runtime,
  cache framework, or broad ProcessKit testing matrix. Do not delete shared
  host/process infrastructure still used elsewhere.
- Preserve S1/O1 behavior unless this amendment explicitly changes it. Any
  additional scope, interface, safety guarantee, or material artifact/profile
  contract change returns to approval before implementation.

## Approval record

The human was shown this full spec path, its immutable-artifact outcome and
debug/CSR/rooting/artifact-lock scope, and was asked: **“Do you approve this
spec amendment?”** The human answered exactly: **“approved”**. The human
separately approved the outline recorded at
`docs/superpowers/plans/2026-11-08-issue-1893-immutable-sandbox-artifacts.md`;
implementation is authorized only within both approved planning artifacts after
they are committed. This does not approve helper bootstrap/cache publication,
Python, additional scope, or merge. No ADR or other enduring documentation
changes occur until conformance review.

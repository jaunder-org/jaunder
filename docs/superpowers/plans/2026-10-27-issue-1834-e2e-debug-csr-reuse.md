# Issue #1834: Debug CSR reuse implementation outline

> Execute with `jaunder-iterate`; delegate through `jaunder-dispatch`. Outline
> needed because the bundle GC root and mutable host binaries share
> checkout-local lock ownership that must outlive browser/server teardown.

## Execution summary

The [approved spec](../specs/2026-10-27-issue-1834-e2e-debug-csr-reuse.md) is
authoritative. First prove one complete ordinary E2E run can embed the new Nix
DEBUG bundle while retaining artifact ownership through teardown. Then prove
failure/invalidation behavior and unchanged-input reuse before final review.
Existing release/visual/interactive paths, test populations and gates stay in
scope only for preservation; no new cache protocol or scheduler.

## Task outline

- [x] **1. Deliver the first complete debug-bundle E2E path.**
  - Producer owner: parameterize the existing WASM producer with
    `CARGO_PROFILE=dev` for dependency/final builds and the `debug` artifact
    path; expose `csrDebugBundle` using the shared postprocessor/source
    predicate. Existing release definitions and consumers keep their current
    defaults.
  - Consumer owner: ordinary E2E selects validated explicit caller input or
    admits, realizes and validates `.xtask/csr-debug-bundle`. Pass its absolute
    resolved root during host server compilation; do not modify `target/site`.
    Visual updates retain their current release production and inherited
    override policy.
  - Ownership: the prepared host-artifact value retains the lock acquired before
    selection; orchestration retains that value until all lifecycle teardown is
    complete. Preserve E2E's existing one-preparation flow; if helper nesting is
    introduced, pass held ownership rather than reacquire the lock. Preserve
    appropriate ownership duration for other callers, including sandbox. Carry
    ownership on error paths; caller-owned artifact retention remains explicitly
    outside our checkout lock's guarantee.
  - Split: Nix producer edits (`nix/packages.nix`) and Rust consumer edits
    (`xtask/src/steps/host_server.rs`, `e2e_local.rs`, existing prerequisite
    helpers) may proceed concurrently against this output/root contract. The
    Rust owner integrates shared helper/caller changes; one writer per file. No
    broad or heavy verification runs concurrently. Do not build a new general
    abstraction.
  - Verification: evaluate dev-profile configuration and native-system exports;
    realize/validate the complete debug bundle, then execute
    `devtool run -- cargo xtask e2e-local posts.spec.ts:139` with Chromium,
    SQLite and one worker. Observe the existing rendering, lifecycle and panic
    checks. Stop on debug/profile, source-admission or embedding mismatch before
    expanding the proof matrix. This is the cross-module tracer, not
    certification.

- [ ] **2. Establish freshness and lifetime failure behavior.**
  - Depend on the integrated producer/consumer path; keep one integration owner.
    Use existing source-identity probes and behavioral test conventions, without
    a second source inventory or mock-only forwarding assertions.
  - Verify the spec's named source/producer/Cargo/toolchain/shell/style probes
    change the final debug derivation, and the unrelated-note control does not.
    Separate source-identity proof from relevant untracked refusal,
    intent-to-add inclusion and tracked-dirty inclusion. Restore every transient
    probe input.
  - Verify missing/corrupt explicit input and failed automatic realization with
    an older root prevent consumer compilation/startup. Verify competing
    cooperating preparations cannot replace the active root or host binaries;
    handled failure/interruption tears down owned children before releasing the
    guard and permits a subsequent invocation. No cache purge or scheduler.
  - Cover unchanged release output selection, visual mode, browser selection,
    standalone build-csr, sandbox and test-local through affected existing tests
    and source/identity checks; add regressions only for uncertain behavior.

- [ ] **3. Prove warm feedback and complete the delivery review.**
  - Repeat the same ordinary focused command unchanged after the first
    successful run. Verify identical debug output selection without packaging
    realization, while a new database/server/collector/capture/browser lifecycle
    still runs. Compare stage timings with #1833's successful observations; no
    new baseline, cached test verdict or universal speedup claim. Report
    first-miss cost and retained-space tradeoff, and distinguish native
    execution from other-system evaluation (especially Darwin).
  - Update CONTRIBUTING and the architecture description to match automatic
    debug selection, explicit input responsibility and lock/root lifetime.
    Review the integrated deliverable through the normal independent Standards
    and Spec/conformance boundary; enforced hooks and authoritative CI remain
    unchanged. No duplicate full local validation merely to repeat CI.

## Risk checks and acceptance mapping

- Task 1: native DEBUG profile/shared postprocessor, release preservation,
  build-time embedding, complete validation and lifetime ownership.
- Task 2: all named invalidation/admission controls, explicit and automatic
  failure refusal, competing ownership and handled interruption/recovery;
  preserve other command/profile paths. Admission alone never proves identity.
- Task 3: real same-scenario warm repeat, fresh consumer state, honest timing
  and platform limits, maintained operating docs and unchanged certification
  policy.
- No unlocked ownership handoff, nested lock acquisition, stale/release fallback
  or copying into `target/site`. Keep the approved spec's caller-owned-input
  exception distinct from Nix-owned DEBUG profile/source guarantees.
- Implement only after this outline is approved. Merge/deployment approval
  remains separate; the unrelated pre-existing #1804 spec is not part of this
  issue.

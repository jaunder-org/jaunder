# Immutable sandbox and e2e build artifacts — #1893 outline

> Approved outline for execution with `jaunder-iterate` after both planning
> artifacts are committed. Needed for the durable Nix artifact boundary,
> build/source identity, consumer lifetime roots, and sandbox/e2e concurrency
> contract. Governing spec:
> `docs/superpowers/specs/2026-11-08-issue-1893-immutable-sandbox-artifacts.md`.
>
> **Approval record:** Parent presented this full outline path, summarized its
> four ordered tasks, and explicitly stated that approval permits committing the
> planning artifacts and implementing within their boundaries. The human
> answered exactly: **“approved”**. This does not authorize work outside the
> approved spec and outline.

## Execution summary

First prove one normal, active-checkout Nix selection yields the matching debug
Jaunder/seed executables with embedded debug CSR and roots their outputs for the
consumer lifetime; preserve the existing e2e release-CSR selection as a second
selection, not a new runtime CSR interface. Then migrate sandbox and e2e-local
independently from shared mutable artifact publication to exact selected paths,
removing only their artifact-lock use. Finally simplify ProcessKit integration
to its supported group stop/report plus required application signal driving and
child wait, and verify lifecycle compatibility and actual removed machinery.
Nix/store ownership replaces checkout artifact serialization; app-owned
workspace/runtime locks remain.

## Task outline

- [ ] **Task 1: Select and root a source-consistent immutable artifact set.**
  - **Contract:** Extend the existing Nix package factories/outputs in
    `nix/packages.nix` using the existing Jaunder, `test-support`, CSR-bundle
    and rooted-selection mechanisms. Provide narrowly scoped debug Jaunder and
    seed outputs composed with the selected embedded CSR; retain the current e2e
    debug-CSR default and release-CSR selection. Keep the consumer-facing result
    to exact server and seed executable paths from one source/profile-consistent
    set. Private Nix attributes may compose existing factories; do not introduce
    a custom artifact manifest/protocol, runtime CSR path, or generic framework.
    Relevant tracked checkout inputs must affect selection; relevant untracked
    inputs must follow the existing admission policy or fail explicitly.
  - **Risk/verification:** Build a smallest real artifact-selection tracer with
    the normal Nix path; inspect exact selected store paths, embedded-CSR
    choice, active-source behavior, and rooted-output lifetime. Check normal
    package builds for each required selection. Exercise cancellation of an
    in-progress Nix request through the existing interruptible host-process
    pattern; prove no selected output is used after failed/interrupted
    selection. Keep tests about this application selector/consumer contract, not
    Nix internals.

- [ ] **Task 2: Migrate sandbox preparation and retain lifecycle behavior.**
  - **Contract:** Replace mutable Cargo/application-output preparation in
    `ux-sandbox/src/preparation.rs` with interruptible selection of Task 1's
    exact immutable paths, preserving server-mode debug host/seed defaults and
    seed subprocess (`test-support seed-sandbox-profile`). Retain Just's
    existing helper bootstrap unchanged. Remove only sandbox's
    `.xtask/host-artifacts.lock` acquisition. Preserve application-owned
    signals/status, readiness, metadata, workspace/storage admission and locks,
    reset/recovery, legacy schemas and digest/generation fencing. Keep the
    existing runtime CSR interface unchanged (CSR remains embedded in the chosen
    server).
  - **Risk/verification:** Focus public-path checks on disposable and named/demo
    startup, readiness and status, reset/recovery and generation compatibility,
    process interruption, and lock ordering. Include a failing/interrupted
    artifact-selection case before storage/server work. Confirm the Nix daemon
    is not treated as an owned child and no artifact lock is held across the
    application lifetime.

- [ ] **Task 3: Migrate e2e-local without disturbing other artifact owners.**
  - **Contract:** Change only `xtask/src/steps/host_server.rs`'s e2e artifact
    selection/consumer to use the same immutable server/seed output contract,
    preserve debug defaults and the current release-CSR option, and remove only
    e2e-local's shared mutable artifact publication/lock lifetime. Preserve its
    interruptible consumer teardown and existing e2e gates. Leave
    `xtask/src/steps/test_local.rs`, the shared artifact-lock facility, direct
    manual `just build-csr`/Vite `target/site`, and seed subprocess boundary
    unchanged.
  - **Risk/verification:** Exercise source admission for tracked edits and
    explicit refusal/admission of relevant untracked inputs; ensure exact paths
    and GC roots survive through consumer teardown and are released afterward.
    Run a focused existing e2e-local path appropriate to the changed consumer;
    do not broaden into the independent full e2e matrix unless its existing gate
    requires it.

- [ ] **Task 4: Thin ProcessKit integration and integrated conformance.**
  - **Contract:** In `ux-sandbox/src/session.rs`, use ProcessKit's supported
    group teardown/report instead of redundant membership polling/retry where
    documented behavior supports it. Keep only application signal distinctions,
    status, readiness/stopping publication, concurrent direct-child wait/reap
    and lock-release ordering. Retain a few meaningful app-integration cases; do
    not recreate the six-operation fault matrix or a generic retry framework. Do
    not claim a ProcessKit bug from an instantaneous `R` state, or strengthen
    platform guarantees.
  - **Risk/verification:** Review actual production deletions against the
    approved simplification criterion, not line movement. Confirm the original
    lifecycle/signal contract and two independent workspace JSON decode-error
    tests remain. Perform separate internal reviews of (1)
    architecture/simplicity and (2) spec conformance against the complete diff
    and actual proof. Resolve findings before fresh authoritative proof; the
    prior 6,061-test coverage result was RED with two JSON gaps and is not
    certification for this tree.

## Risk checks and gates

- Preserve immutable Nix output permissions. Do not edit
  `server/src/build_staging.rs` or `server/tests/build_script.rs`; #1923 owns
  the local-copy owner-write repair and fresh-local-copy semantics.
- Do not remove `test-local`'s separate artifact-lock use or shared lock
  facility. Do not alter `Justfile`'s helper Cargo bootstrap, add helper
  cache-publication work, decide Python, add a new dependency/fork, or add a
  foreign-native matrix.
- Production work begins only after both approved planning artifacts are
  committed. This commit records the planning boundary; it does not implement
  the feature. The parent owns the next execution assignment.
- After implementation and review, run the existing focused checks and one fresh
  authoritative coverage/Nix/CI proof on the new clean intended tree using
  project-supported commands and established timeouts. Preserve honest exits and
  parked artifacts; do not bypass gates or infer a pass from old-tree evidence.
  Keep the existing e2e gates authoritative and independent.
- After conformance review, project enduring decisions into the current
  ADR/architecture view, then delete completed planning documents under the
  normal artifact lifecycle.

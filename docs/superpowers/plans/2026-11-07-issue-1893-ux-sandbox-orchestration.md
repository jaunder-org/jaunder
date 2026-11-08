# #1893 UX sandbox orchestration implementation outline

> Execute with `jaunder-iterate`. Required for the approved process-ownership,
> public-command, and lifecycle storage/concurrency boundary. Scope is the
> approved spec
> `docs/superpowers/specs/2026-11-07-issue-1893-ux-sandbox-orchestration.md`.
> Human O1 approved this outline and implementation (`1893-o1`, request
> `0a5e9118-864e-47ab-9247-ebaecf0aaa74`) after the approved S1 design; the
> planning contract was committed before implementation. Whole-branch review, D2
> acceptance and final delivery gates remain Lead-owned.

## Execution summary

Build the smallest complete public tracer first: from a normal checkout,
`just sandbox` builds the helper, prepares and selects protected server
artifacts under the artifact lock, initializes an empty disposable workspace,
reaches actual Jaunder runtime-info/identity/readiness, then safely stops and
reaps before cleanup. Prove processkit’s descendant-ownership assumption with
focused native fixtures alongside the actual app identity/readiness proof; this
is a feasibility check, not foreign-platform certification. Expand lifecycle and
command behavior, then retire the old xtask entry only after replacement parity.
Preserve the approved direct async Tokio/processkit owner, seed subprocess, lock
contracts, and D2 overall-simplicity acceptance.

## Task outline

- [x] **Task 1: Public disposable-server tracer works from a normal checkout**
  - Add the root-workspace `ux-sandbox` binary and one public `just sandbox`
    recipe with positional arguments and exact argv forwarding. Bootstrap the
    helper to distinct `target/debug/ux-sandbox`; under
    `.xtask/host-artifacts.lock`, prepare/select server CSR
    (`just build-csr debug`), Jaunder, and test-support debug outputs, then
    release the lock before child execution. Initialize empty disposable storage
    (no manual prebuild or manual initialization), serve the actual Jaunder
    executable, parse/validate runtime-info identity and address, and establish
    HTTP readiness.
  - Direct async Tokio/processkit supervision owns the server and descendants.
    Stop safely and prove containment/reap before removing disposable storage.
    Keep the current xtask route operational until later replacement parity.
  - **Verification:** Public Just-path smoke with exact argv; from a
    clean/normal checkout build and selected-artifact proof; actual runtime-info
    identity/readiness; empty init; cleanup-after-reap. Use focused native
    controllable process-tree fixtures to establish the processkit
    containment/reap assumption, without requiring a cross-platform
    certification gate before further work. Confirm helper bootstrap
    output/build-script effects do not mutate protected outputs.

- [x] **Task 2: Named lifecycle, reset/recovery, and command admission preserve
      storage contracts**
  - Add named empty/standard/demo profiles, creation init, resume without
    init/reseed, retained `test-support seed-sandbox-profile` subprocess, reset
    of existing names with explicit/recorded profile, and the specified
    two-rename recovery. Preserve name/path/symlink fences, versioned atomic
    metadata, executable digest/generation identity, workspace-local profile
    metadata, and stable/ephemeral lock locations.
  - Add allowlisted operational command mode with exact argv/status. Grammar is
    one lowercase ASCII/digit name component; allowlist: `site-config`,
    `user-create`, `app-password-create`, `user-invite`, `smtp-test`, `backup`,
    `websub`; reject lifecycle commands, `--storage-path`, and `--db`.
  - **Lock contract:** Server lease precedes workspace transitions. Stopped
    command admission obtains lease, exclusive workspace recovery/admission,
    validates path/metadata, and takes `<workspace>/runtime.lock`; retaining
    lease+runtime guard, transitions exclusive to shared and only then releases
    lease. Hold runtime+shared through child. Live command requires ready
    metadata and matching digest before storage access; hold shared through
    child, without lease/runtime lock. Reset requires lease and runtime-lock
    check. Shutdown retains lease through exclusive cleanup barrier; cleanup
    follows process-tree reap.
  - **Verification:** Focused behavior for name/path/symlink validation,
    init/resume/seed, all specified reset recovery states and failed
    publication, metadata/fingerprint, stopped/live admission,
    readiness/identity mismatch before storage access, unmanaged runtime lock,
    exact argv/status and child-lifetime locks. Verify lock order and no
    self-deadlock; no partial publication or premature cleanup.

- [x] **Task 3: Command-mode builds and contention integrate with existing e2e
      ownership**
  - Extend protected preparation to command mode: while holding
    `.xtask/host-artifacts.lock`, build/select debug Jaunder only (no
    CSR/test-support); release before child use. Verify both modes’ preparation,
    artifact selection, and serialization against the actual shared artifact
    lock. Keep helper bootstrap outside the protected lock only while
    distinct-output/build-script inspection supports that assumption; return any
    contradiction for approval rather than adding an unapproved cache/lock
    design.
  - Preserve e2e-local artifact-lock ownership through consumer teardown
    unchanged.
  - **Verification:** Mode-specific build/output checks, contention proof
    between helper and e2e-local protected consumers, and evidence that
    e2e-local retains its existing lock lifetime. Include the cold helper build
    in command-mode cost and the integrated D2 simplicity assessment.

- [x] **Task 4: Signal/failure semantics, parity cutover, and accepted-design
      maintenance**
  - Complete graceful drain and failure cleanup under the direct async owner.
    First SIGINT/SIGTERM begins drain; additional SIGINT may force earlier;
    repeated SIGTERM joins drain without escalation. Automatic escalation is
    exactly 10 seconds. Preserve initiating status (130/143) and child status;
    keep `jaunder shut-down`’s independent 30-second wait/no-escalation
    behavior. Do not release lifecycle locks or clean storage/metadata before
    descendants are contained/reaped and the cleanup barrier is held.
  - After Tasks 1–3 provide replacement parity, remove the sandbox xtask
    CLI/parser/dispatch/result-report protocol and only sandbox-specific
    bridges/adapters with no remaining consumers. Preserve `host_server`,
    `host_signals`, and `process` where e2e-local uses them. Maintain the
    numberless **proposed** ADR and architecture projection, preserving their
    proposal/current-truth distinction. Apply only status/reference updates
    authorized by the established lifecycle; no feature-side ADR numbering,
    acceptance, or promotion. Serialized post-merge automation owns promotion.
    Do not rewrite historical ADR Decision text.
  - **Verification:** Focused signal/status/deadline and unexpected-parent-exit
    tests with actual descendants; cleanup-after-reap assertions. Verify the
    full four acceptance clusters below, unchanged e2e-local behavior, and
    independent integrated review finding lower total package/runtime complexity
    rather than relocation. Follow normal focused proof and repository gates
    without duplicating their procedures here.
  - **Implementation handoff:** Retirement, current-doc projection and focused
    native/public proof are complete. This checkbox records the bounded worker
    implementation, not D2 acceptance or project completion. Lead still owns
    independent whole-branch conformance/D2 review, corrections and final
    repository/delivery gates; the working spec and outline remain for that
    review.

## Acceptance and governing constraints

The approved spec has **four acceptance clusters**:

1. Exact argv/modes/profiles/allowlist, single package and forbidden-dependency
   absence, distinct output, mode-specific builds/locks, unchanged e2e-local
   ownership — Tasks 1, 2, 3, 4.
2. Name/path/symlink, init/resume/reset/recovery, metadata/fingerprint,
   lock/admission, runtime identity/readiness, seed subprocess, no partial
   publication/premature cleanup — Tasks 1, 2, 4.
3. Signals/status, 10-second automatic deadline, SIGINT early-force vs
   repeated-SIGTERM drain, actual descendant containment/reap and cleanup
   barrier without broader guarantees — Tasks 1, 4.
4. Independent proof of lower total package/runtime complexity, not relocation —
   Task 4.

The outline/implementation approval and prior planning-contract commit gates
were satisfied before implementation. Platform compatibility follows
`CONTRIBUTING.md`: production validation on x86_64 Linux, periodic committed
Apple Silicon macOS development checks, and no ARM Linux native-delivery gate.
Task 1’s focused native fixtures are feasibility evidence, not an extra platform
certification gate.

## Self-review

The slices are behavior-sized and ordered around the riskiest assumption. Task 1
is a complete public tracer from a normal checkout, not a probe dependent on
deferred preparation. Task 3 extends rather than postpones command-mode
build/lock proof. Task 4 preserves proposed ADR/projection lifecycle without
feature-side promotion. Scope retains the approved 10-second grace, D2
simplicity criterion, seed subprocess, e2e-local ownership, and existing
shutdown contracts; it introduces no generic process framework, registry,
private runtime abstraction, or broader guarantee.

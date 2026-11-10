# Project-controlled workflow tracing implementation outline

> Execute with dev-cycle-iterate only after separate approval. Needed because
> passive owner integration, failure isolation and diagnostic lifetime cross
> independently owned boundaries. **Revised draft, not executable.**

## Execution summary

First prove useful passive recording inside one existing owner without changing
its execution or cleanup. Extend to the revised spec's controlled callsites,
then local bundle/export and native CI publication. No Just investigation,
patch, interception or full native-root parity work remains in this outline.

Governing spec: `../specs/2026-11-10-issue-1859-always-on-workflow-tracing.md`.
R9 Manager background `b72fc8dd-1a23-4cb0-a2e6-1a09a1537494`: “No, we're not
going to patch just, upstream or otherwise. We just need good tools for
capturing tracing data at the boundaries we control.” Early receipts: “Allow
bounded early receipts”; activation: “Yes, disclosed automatic capture”. This
resets scope, not final approval. R6 status/gap policy, R7 native CI expiry and
local worktree-lifetime/export decisions in the spec remain unchanged. Prior
spec-only approval `ea7450e0-a2a8-4bc0-af71-906690baedc5` and outline-draft
authority `3786e460-8f3b-4a85-9d80-108f5457aef8` grant no implementation here.
Pre-R9 bytes/hashes are preserved under `.xtask/issue-1859-planning/pre-r9-*`.
Both revised documents need proportional independent review and explicit
approval.

## Task outline

- [ ] **1. One controlled owner writes useful receipts without owning
      processes.**
  - Contract: reuse existing trace serialization/publication code via a small
    passive interface in the current trace-writer owner. Rust calls it
    in-process; existing shell owners use bounded write-only operations. No
    child execution, waits, signals, deadlines, process groups or terminal
    changes. Preserve owners' existing exec/trap behavior; do not add a waiting
    wrapper to get an exit code.
  - Write separate start/completion records and participant-owned files by
    temporary write/rename in private `.xtask/workflow-traces/<id>/`. Use the
    spec's bounded fields, not generic payloads. An opaque context is passed
    only by an actual observing owner; no global active-root/dedup/PID registry.
    Entry-only, lost context or missing completion stays explicitly unknown.
  - Verification: tiny owner-interface fixtures cover observed success/failure,
    nested/parallel records, unavailable/partial publication and abrupt death.
    Mandatory failure/interruption cleanup lasts **over two seconds** and
    completes exactly once with recording available or unavailable; original
    status and sole lifecycle ownership remain intact. Prove one bounded real
    owner path later, before coverage expansion; no benchmark campaign.

- [ ] **2. Activate disclosed capture with privacy-safe early observations.**
  - Contract: enable automatically at the revised coverage table's project-owned
    callsites in the trusted checkout/CI; disclose scope/location/limits in
    current developer guidance and CI output. No per-run opt-in, global
    activation, PATH replacement, native CLI hooks or capture for
    unrelated/excluded operations. Existing Rust entry/return, safety
    classification/reconciliation, focused lifecycle and E2E script phases are
    preferred seams. Just callsites record only what their existing body can
    safely observe, without graph restructuring.
  - Early records require actual entry into a controlled boundary and owned
    output availability. Before classification permit only ID, start/elapsed,
    fixed stage/outcome and capture health. No caller text/path/argument
    fingerprint; no detailed excluded-operation events after resolution. No
    global fallback sink when checkout/output ownership cannot be established.
  - Verification: per-table-row activation checks and excluded/raw/direct-Just
    cases prove absent native root/pre-resolution observation remains unknown.
    Secret-shaped argv/env/path/content fixtures never enter receipts. Known
    selection/skips require an owner observation; missing records prove nothing.
    Preserve command order, arguments, native deduplication and existing
    signals; test observers, not a reimplemented engine.

- [ ] **3. Keep correlation and retained analysis honest.**
  - Contract: source/dirty, tool/store and CI attempt identities use allowlisted
    facts already available to owners. Unknown identity stays unknown. Explicit
    parent links join only observed relationships; preserve clock frames and
    overlaps. Cached producer artifacts are not current execution. Keep current
    analyzers and their artifact/report inputs, documented
    analyze/run/boot-phase outputs and serial/single-worker measurement
    controls, not generic envelopes.
  - Verification: focused fixtures distinguish reuse/current tests, dirty
    inputs, missing provenance, concurrent phases and separate backend/browser
    attempts. Reuse per-attempt join, malformed/missing-evidence and
    boot-frame/median regression fixtures; do not re-test COTS internals or
    change required verdicts.

- [ ] **4. Retain local bundles and publish ordinary CI diagnostics.**
  - Contract: local private ignored bundles outlive runner count pruning, with
    no age expiry/collector. Copy only essential approved payloads, not
    recursive logs. Operator explicitly exports selected evidence and verifies
    readback before separately requesting worktree removal; markers grant no
    export/removal proof.
  - CI uses the existing upload action with native `retention-days: 14` for
    ordinary complete/incomplete bundles. Link `expires_at` and native
    run/job/attempt/step references. Existing aggregate results or exact-attempt
    jobs API readback can explain observed status; unknowns stay unknown and do
    not stop expiry. Owner exports selected incident/uncertain evidence before
    expiry. Failed export plus expiry is visible loss, not conditional
    indefinite protection.
  - Verification: owned fixtures cover local age/pruning survival, partial
    export, test-owned removal/readback, native CI retention configuration,
    incomplete status and export/expiry loss. Capture/upload pressure visibly
    reports gaps without changing workload exit or required artifact/cleanup
    failures. Later bounded CI proof checks actual artifact access, not
    guaranteed future storage.

## Risk checks, ownership and acceptance

All spec coverage/activation rows map to Task 2; failure/lifecycle proof to 1/4;
privacy/provenance/analyzers to 2/3; retention/export to 4. Focused proofs
precede ordinary iteration/ship gates later; no tests or gates are authorized
here. Preserve backend/browser populations and use `#[apply(backends)]` where
shared storage behavior is exercised. Enduring docs/any necessary narrow ADR
accompany actual slices; raw diagnostic evidence remains ignored or in approved
storage.

Manager-accepted split stands: #1859 owns passive receipts/privacy/bundles and
diagnostic CI; #1898 owns successors/removal; actual owners retain behavior and
lifecycle. Serialize shared Just/safety/focused/E2E/CI callsite edits only;
prefer independent foundations then settled successors. Keep host diagnostics
out of hermetic producer inputs. No #1898 document edits or blanket
tracing-before- retirement prerequisite. No supervisor, workflow engine,
scheduler/#1753, production tracing, uncontrolled cleanup or secret capture.

## Evidence and limits

Reuse inspected project seams at landed context
`8b4095da3f0d5e8cbb982eea3045ccc17ef3fe8d`: `Justfile`, safety/focused owners,
`scripts/e2e-workflow`, existing trace writer and `.github/workflows/ci.yml`.
Existing launcher supervision is not a passive API and must not be added around
workflows. Native CI artifact expiry remains independent of receipt status. R9
deliberately accepts unobserved native Just lifetimes; no upstream delivery,
patch, engine parity or native-root proof is needed. This draft claims no new
implementation, runtime feasibility proof, measurement or platform validation.

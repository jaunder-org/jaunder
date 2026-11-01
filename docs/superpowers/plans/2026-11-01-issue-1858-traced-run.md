# Issue #1858 implementation outline — standalone `traced-run`

> Proposed implementation decomposition for review; no implementation is
> authorized by this draft. The proposed design is in the companion spec and
> remains subject to human approval.

## Scope

Extract only `devtool run` into a development-shell-provided standalone runner.
Retain unrelated `devtool` uses and host-only `xtask` orchestration. Implement
opt-in Perfetto-compatible Chrome Trace Event JSON using the exact CLI, launcher
re-entry guard, correlation, shared monotonic clock, and single-root ownership
recommendations in the spec. Replace scoped in-repository runner consumers.
External policy/skill routing is Manager-owned. No permanent duplicate
interface, whole-command rewrite, `just` migration, or #1857 merge.

## Tasks

- [x] **1. Implement approved placement and compatibility contract.** After
      approval, amend ADR-0028 to reconcile standalone host/devshell placement
      and pre-Nix launcher availability with its sandbox/host boundary. Extract
      the direct single-program runner and tests; package it in the devshell,
      retaining unrelated `devtool` operations. Preserve shell refusal,
      output/result/error schema, timeout/signal/exit semantics, history bound
      and honest status.
- [x] **2. Implement process spans and root finalization.** Add opt-in
      `--name LABEL`, normal JSON-result mode, and `--launcher` inherited-stream
      mode as specified. Implement opaque run/parent IDs, explicit participating
      runner/launcher descendants only, per-writer event files, shared monotonic
      origin, atomic root merge/publication, concurrent run isolation and
      documented abrupt-termination limitations. Never instrument or claim
      arbitrary inherited children.
- [x] **3. Integrate launcher and perform coordinated interface migration
      (in-repository integration; external cutover pending Manager
      confirmation).** Before checkout discovery/Nix, `scripts/cargo-xtask` uses
      a dedicated private re-entry guard (distinct from trace context): on first
      entry it execs the already-built PATH runner as
      `traced-run --launcher --name cargo-xtask -- <absolute-script-path> --inner <original-args>`.
      The supervised executable is the approved shebang script, not arbitrary
      shell `-c`; inner phase consumes/clears the guard, discovers checkout, and
      execs Nix without wrapping again. The guard marks only that supervised
      inner phase, never authorizes skipping wrapper on a later invocation;
      clear it before Nix exec. Preserve original shell refusal, direct
      inherited stdout/stderr, no output capture files or summary JSON for the
      inner launcher, and child status. Direct launcher output is observable
      before completion. If an outer normal captured runner wraps `cargo xtask`,
      the outer runner owns and parks stdout/stderr and its summary; interactive
      output is not promised. The inner launcher joins the outer trace root,
      adds no JSON or duplicate output files. Distinguish trace-root ownership
      from output ownership. Update scoped in-repository consumers. Do not
      remove/switch canonical external `devtool run` guidance until
      Manager-confirmed exact migration readiness; prefer atomic coordinated
      cutover, with only a bounded shared-implementation forwarding route if
      required.
- [x] **4. Prove contracts and inspect one workflow.** Cover capture/result
      compatibility; require output-before-child-completion only for direct
      launcher mode (externally captured output is parked by the outer runner);
      nonzero/signal/timeout/spawn and early launcher errors, malformed/refused
      CLI, recursive participating descendants, uninstrumented arbitrary
      children, shared-clock alignment, concurrent roots, single trace/root,
      ordinary and failure finalization, and abrupt termination honesty. Run the
      filtered
      `cargo xtask test-local -- -p storage site_config_primitives_round_trip`
      workflow and inspect its trace in Perfetto; record useful spans and
      unattributed time without claiming speedup. Follow the approved validation
      ladder.
- [ ] **5. Conformance and artifact retirement (implementation/proof conformance
      accepted; retirement pending exact Manager-confirmed external routing
      cutover so temporary forwarding can be safely removed).** Inspect
      implementation against approved spec and proofs; project enduring
      decisions into current docs/ADRs, retire temporary spec/outline under
      repository artifact lifecycle after conformance, and record remaining
      work. Keep #1857 unmerged and use it only as selectively applicable
      experimental evidence.

## Conformance state (2026-11-02)

Tasks 1, 2, and 4 are implemented/proven. Correction commits are
`74b1c0b2df707ae21e62436d741bcdb314b0996e` (tree
`7edc9821f7efb66a96d53ced966ab21ec00ecae4`) and
`594e0d20d15d6c7697335206b3e179ff7c327d8b` (tree
`9be669cf19a690dcc2f054a183c4dbdb4e6f9bb9`). The final package build, packaged
filtered workflow, and exact-trace Perfetto import/inspection passed; see
ignored `.xtask/issue-1858-corrected-proof.md`. The workflow retained no exact
nextest census, internal work is opaque by design, and this is not a performance
claim. Native macOS remains untested. Terminal review reported no findings.

In-repository integration is complete, but external command-routing migration
and deployment/activation are still Manager-owned and pending. Keep the bounded
`devtool run` forwarding entry point operational. Because the approved spec
makes that forwarding route removable only after exact external migration
readiness, final artifact retirement is not yet appropriate; do not report the
spec/outline lifecycle complete. ADR-0028, `docs/ARCHITECTURE.md`, and
`docs/observability.md` already contain the enduring contract and rationale. No
further enduring-doc gap was found. See ignored
`.xtask/issue-1858-conformance-handoff.md` for the full evidence inventory,
limitation details, lifecycle rationale, and next owner/action.

## Review boundary and current checkout evidence

Checkout: `/home/mdorman/src/jaunder/issue-1858-traced-run`; branch
`issue-1858-traced-run`; HEAD `f4868289cbb1fb28790cd5232127efa02173874f`.
Read-only `devtool run -- git status --porcelain=v1 --branch` reports exactly
two untracked files—the spec and this outline—and no staged or tracked
modifications. These drafts are the only observed working-tree changes; do not
characterize the checkout as clean. The status output is parked at
`.xtask/run/1793537864789-439914.out`; HEAD at
`.xtask/run/1793537864790-439915.out`; cwd at
`.xtask/run/1793537864790-439916.out`. No code/build/test/viewer/tracker/commit
operation was performed for this correction.

#1858 remains subject to its existing tracker state; this draft does not change
it. PR #1857 remains OPEN/ready-to-land and explicitly NO MERGE; its evidence
does not authorize merging or make it a dependency. Approved-document
hook-backed commits and implementation require explicit subsequent
authorization; this packet requests both precisely, with implementation only
after spec/outline approval. Retain this branch/checkout and existing
experimental artifacts.

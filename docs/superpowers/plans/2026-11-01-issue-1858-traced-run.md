# Issue #1858 implementation outline — standalone `traced-run`

> Approved implementation outline, maintained during bounded corrections.
> Earlier review packets below are historical provenance, not current checkout
> inventory or a renewed approval request.

## Scope

Extract only `devtool run` into a development-shell-provided standalone runner.
Retain unrelated `devtool` uses and host-only `xtask` orchestration. Implement
opt-in Perfetto-compatible Chrome Trace Event JSON using the exact CLI, launcher
argv-only re-entry, correlation, shared monotonic clock, and single-root
ownership recommendations in the spec. Replace scoped in-repository runner
consumers. External policy/skill routing is Manager-owned. No permanent
duplicate interface, whole-command rewrite, `just` migration, or #1857 merge.

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
      a cooperative private argv-only re-entry (distinct from trace context): on
      first entry it execs the already-built PATH runner as
      `traced-run --launcher --name cargo-xtask -- <absolute-script-path> --inner <original-args>`.
      The supervised executable is the approved shebang script, not arbitrary
      shell `-c`; inner phase consumes `--inner xtask`, discovers checkout, and
      execs Nix without wrapping again. The argv entry marks only that
      supervised inner phase, never authorizes skipping the wrapper on a later
      invocation; no environment guard or runner basename coupling remains.
      Preserve original shell refusal, direct inherited stdout/stderr, no output
      capture files or summary JSON for the inner launcher, and child status.
      Direct launcher output is observable before completion. If an outer normal
      captured runner wraps `cargo xtask`, the outer runner owns and parks
      stdout/stderr and its summary; interactive output is not promised. The
      inner launcher joins the outer trace root, adds no JSON or duplicate
      output files. Distinguish trace-root ownership from output ownership.
      Update scoped in-repository consumers. Do not remove/switch canonical
      external `devtool run` guidance until Manager-confirmed exact migration
      readiness; prefer atomic coordinated cutover, with only a bounded
      shared-implementation forwarding route if required.
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

## Current bounded correction state (2026-11-02)

The recognized real-process Perfetto flow correction was committed as
`dedbbf8ada02b00863f42fee9dfe788f9ff7e793`; argv-only cooperative launcher
reentry followed in `12484e2bf6bd8b82c89441256e1e09afdb6f1c0b`. Human feedback
accepted the demonstrated viewer presentation, not merge/cutover or pending
independent code review. Subsequent screenshot diagnosis found that the broad
parent-start/child-end anchors misleadingly crossed process durations. The
bounded entry-anchor correction preserves those earlier traces/screenshots and
real process tracks, but binds flows from a parent-track `recorded child entry`
instant to the child's actual recorded slice beginning. Both anchors project the
child's participation timestamp, not an invented OS spawn time. Real runner
output, imported SQL containment/links and selected arrows are verified under
ignored `.xtask/issue-1858-entry-flow/`; no broader redesign or cutover is
implied.

The packaging correction separates `packages.traced-run` from
`packages.devtool`: narrow runner sources plus shared Cargo resolution inputs,
only `bin/traced-run` in the former and only `bin/devtool` in the latter.
Default/CI shells have one authoritative standalone runner provider; temporary
`devtool run` forwarding remains. Admitted devtool-only implementation edits
preserved the runner derivation while changing devtool; runner edits changed
both, and probes restored bytes exactly. This is not whole-devShell immunity;
tools lock/manifests/configuration/toolchain can still invalidate the runner.
Evidence is ignored under `.xtask/issue-1858-package/` and
`.xtask/issue-1858-package-integration/`, not delivery-ledger source files.

Deployment/activation prerequisite: realize and verify the independent
`packages.traced-run` output and its direct, wrapped and nested launcher
contracts, alongside the retained devtool forwarder. The former bundled devtool
output is not the new runner deployment input. No activation or external routing
cutover is authorized here. Manager owns readiness/cutover; independent review
and final acceptance remain separate. Spec/outline retirement and forwarder
removal remain pending and are not performed in this step.

## Earlier conformance evidence (retained provenance)

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
further enduring-doc gap was found at that earlier boundary; the independent
packaging boundary is now projected above and in ADR-0028/ARCHITECTURE. See
ignored `.xtask/issue-1858-conformance-handoff.md` for the full evidence
inventory, limitation details, lifecycle rationale, and next owner/action.

## Historical pre-approval review packet (superseded inventory)

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

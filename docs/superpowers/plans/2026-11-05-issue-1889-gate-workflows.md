# #1889 — Gate workflow implementation outline

> Execute with `jaunder-iterate`. Needed because the approved change crosses the
> Rust leaf boundary, Just workflows, hook/CI callers and Git/index safety.
> Governing spec:
> `docs/superpowers/specs/2026-11-05-issue-1889-gate-workflows.md`. Human spec
> approval: Manager relay `1d21c0df-6e73-4c89-a40e-ddf5b67d92ba`,
> `issue1889-spec-v2`, “Approve revised design”. Outline approval: Manager relay
> `38808dc8-5ac2-422f-a9fe-3b3843758f43`, `issue1889-outline-v1`, “Approve
> outline and begin”.

## Execution summary

Use the existing Justfile as the one workflow definition, retaining Rust check
algorithms behind only the callable single-leaf boundary Just needs. Prove the
precommit failure/reconciliation seam first; then migrate local and CI callers,
shared Prettier ownership, and the narrow ADR projection. Keep one checkout
writer. This approved outline authorizes the implementation cycle; this
preparation operation itself makes no source changes.

## Task outline

- [x] **1. Prove callable leaves and precommit safety.** Expose only the
      Rust-only checks that Just cannot invoke directly, one leaf per call;
      retain their algorithms and remove no check population. Keep precommit's
      classification, snapshot, staging reconciliation and docs admission safety
      in a focused single-process owner. It must reconcile exactly once after
      host failure, preserve the original failure, and run docs post-request
      state verification even when the Nix request fails.
  - **Verification:** focused fake-command/temp-repository tests for uncertain
    broad routing, staged-path-only safe restaging, tolerated unstaged
    pre-existing untracked/delete/rename state, fail-closed unsafe state,
    reconcile-once after success and failure, original failure preservation, and
    docs admission/postcheck boundaries. Never test against the real index.
  - **Contract:** no gate population selection, sequencing, fail-fast policy, or
    gate-wide report is added to the Rust leaf boundary.

- [ ] **2. Cut over workflows and callers.** Add concise Just recipes for hooks,
      developer workflows, local validation and existing CI lanes; invoke
      current producer/consumer entrypoints and preserve their artifacts,
      verdicts and cleanup. Use #1890's human-approved design-v2 interface:
      default full synchronous `just e2e` owns matrix, reconciliation and
      authoritative server-function coverage, with its exit status
      authoritative. CI may call its minimal producer/reconcile recipes; do not
      create a shared result protocol or Nix-packaged workflow. Preserve
      `e2e-lanes.json`. Reference this interface only after #1890 actually
      lands; its outline is still awaiting approval. Until then, use currently
      landed entrypoints independently—there is no implementation dependency or
      peer import. Update this checkout's hooks, docs and CI to Just. Before
      removing old cargo gate commands/selectors, use a Manager-owned readiness
      checkpoint: Manager confirms the external agent gate selector and policy
      cutover is available and maps each selector to its replacement. The
      implementation writer does not edit external policy/config. Then remove
      obsolete aliases in the same local caller cutover—no indefinite forwarding
      layer.
  - **Verification:** recipe-level fake-leaf tests for Fix/Verify, honest
    nonzero child status, necessary cleanup and required ordering; inspect
    hooks, docs and CI for a single workflow source. Verify independent CI job
    results remain authoritative and `last-result` sidecars are diagnostic only.
  - **Dependency:** readiness confirmation from Manager for external selector /
    policy cutover before removing commands those selectors invoke.

- [ ] **3. Consume #1887 and reconcile architecture.** At component
      implementation start, recheck #1887's live ownership. If it remains
      unowned and the existing Manager authorization applies, claim it In
      Progress and proceed; ask the Manager only if execution ownership is still
      unresolved, not for renewed permission for the approved component. Share
      only the three Prettier definitions and remove the docs-only
      `docsDevtoolBin` source/vendor/build path and consumer. Keep other devtool
      checks unchanged. Add one narrow numberless superseding ADR draft plus its
      `docs/ARCHITECTURE.md` projection; do not edit accepted ADR Decisions.
  - **Verification:** host/hermetic callers use the shared Prettier definitions;
    `static-docs` and `static-code`, producer verdicts and required CI
    aggregates remain distinct and authoritative; review the ADR diff against
    only changed ownership. No sibling imports or takeovers.

## Risk checks

- The Just dependencies express success-path ordering only; use a focused
  single-process safety owner for the required post-failure reconciliation.
- Do not create a Nix-script default, a workflow framework, mirrored Rust/CI
  catalogs, or permanent compatibility aliases.
- #1890 E2E and #1891 coverage remain independent; use #1890's approved
  `just e2e` contract only after it lands and integrate no peer code. No
  artificial dependency, cross-checkout writer, or fourth migration slot.
- Run focused proofs first, then required `precommit`/`prepush` hooks and
  current required CI lanes/aggregates. Do not use broad `validate` as routine
  diagnostics. Delete this planning artifact before PR preparation per the
  repository artifact lifecycle.

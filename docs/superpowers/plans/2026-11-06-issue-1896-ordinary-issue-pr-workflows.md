# Ordinary issue and PR workflows without xtask — implementation outline

> Execute with dev-cycle-iterate. Outline needed because the approved design
> relocates the GitHub observer into a host-only tools package and separates
> observation from approval-bearing land/cleanup operations.

## Execution summary

First record the approved ownership boundaries and architecture projection. Then
prove the riskiest assumption with a focused read-only `tools/pr-observer`
tracer: the reused observer can run outside xtask and still classify the
complete live ruleset/queue evidence. Only after that path is correct should
ordinary issue operations and PR land/unmanaged cleanup move behind thin Just
frontdoors. Remove ordinary xtask commands and forwarding/report consumers
alongside their replacement callers. The observer remains read-only; direct
`gh`/`git` operations own mutations.

## Task outline

- [x] Task 1: Record the approved observer/tool ownership decisions
  - Contract: Add numberless successor/amendment ADR draft(s) in
    `docs/adr/drafts/` for ADR-0087 observer ownership and ADR-0028 host-tool
    placement. Cite the exact accepted spec and approval identity
    `issue-1896-spec-design-v1`; keep existing numbered ADRs immutable. Update
    `docs/ARCHITECTURE.md` in the same change, citing draft paths and preserving
    ADR-0135, ADR-0171 and ADR-0077 contracts. Do not edit generated
    `docs/README.md` or run promoter tooling.
  - Verification: Run the repository's existing ADR-format, document-link and
    architecture-projection checks; Lead review confirms draft citations and
    prose reflect only the approved ADR-0087/0028 changes and leave numbered
    ADRs immutable. Add no new validation mechanism.
- [x] Task 2: Run the focused observer outside xtask
  - Contract: Add the host-only `tools/pr-observer/` package to the `tools/`
    workspace with a read-only watch command. Reuse only the necessary
    poller/evidence classifier; no land, cleanup, generic command dispatch, or
    xtask report envelope. Preserve complete live ruleset contexts, fail-closed
    incomplete/empty evidence, exact-head history, `ready-to-land`, and distinct
    failed-check, observer-error, timeout, pending,
    blocked/conflicted/stale/closed-unmerged, merged, and same-head
    dequeued/ejected outcomes.
  - Verification: Offline/pure tests cover late aggregate appearance,
    empty/incomplete gate, required failure versus transport failure, timeout,
    readiness, head change, queued/merged state and same-head dequeue/ejection.
    A real focused invocation confirms Just can launch the host tool and watch
    cannot mutate GitHub.
- [ ] Task 3: Move ordinary issue workflows to explicit Just/gh frontdoors
  - Contract: Provide one documented Just entrypoint per issue discovery, create
    and edit operation. Keep type, labels, milestone, Priority, Backlog
    metadata, explicit dependencies and body readback/mangling checks; do not
    add a general runner or reimplement GitHub semantics. Retire the ordinary
    issue CLI/helper/report callers only after each operation has a replacement.
  - Verification: Focused tests or command-level fixtures prove complete
    metadata and body readback, native dependency handling, and visible failure
    on partial/malformed/API responses; a repository-wide caller search confirms
    no ordinary issue xtask CLI/report consumer remains after migration.
- [ ] Task 4: Move PR land and unmanaged cleanup to guarded Just/gh/git
      frontdoors
  - Contract: The caller first obtains and records human approval scoped to
    PR/head, passes that approved SHA, and the land recipe checks it against the
    live PR head before `gh pr merge --auto --match-head-commit`. Cleanup is
    invoked only after the caller positively verifies unmanaged ownership;
    managed or uncertain ownership must not invoke it. Before safe
    detach/delete, preserve merged base `main`, exact local branch/HEAD to PR
    head, clean-tree and captured-head ancestry checks. Retire ordinary xtask
    land/cleanup command dispatch and report forwarding with the final
    consumers. Keep the ADR promoter untouched.
  - Verification: Tests/fixtures cover approval-scoped exact-head drift/refusal,
    wrong base, dirty tree, non-ancestor, managed/uncertain caller refusal, and
    successful safe cleanup sequencing; a repository-wide caller search confirms
    no ordinary PR xtask CLI/report consumer remains. Approval remains a
    human/workflow-owner prerequisite, not a Just-enforced fact.

## Risk checks

- Keep the observer the only custom polling/evidence algorithm; Just owns no
  poll loop and observer owns no mutation.
- Do not remove the final xtask caller before its replacement frontdoor and
  focused proof exist; at completion verify no ordinary issue or PR CLI/report
  caller remains.
- Exact human approval is `issue-1896-spec-design-v1` /
  `f83b8b7a-3af7-4396-9dc5-347f089eec3c`; changed contract or accepted
  implementation tree requires Lead review, and any changed approval inputs
  require renewed human approval.
- ADR successor drafts and `docs/ARCHITECTURE.md` projection are future
  implementation deliverables under the approved boundaries, not current
  accepted ADR edits; serialized promotion remains CI-owned.
- Cleanup and issue claim release follow `jaunder-ship`; Manager-assigned work
  preserves branch and checkout and hands off inventory rather than cleaning up.

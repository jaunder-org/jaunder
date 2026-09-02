# Conflicted Promoter Replacement Implementation Outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` for independent code,
> documentation, and external-skill slices. This outline exists because the
> controller changes a durable Git/GitHub protocol with concurrency,
> authentication, and crash-recovery invariants.

## Scope

In:

- Replace a positively conflicted immutable promoter attempt from fresh `main`.
- Resume every intent/delete/close/generate/push/create/arm interruption from
  durable GitHub/Git state.
- Abort an unarmed generated PR that becomes stale before its publication
  linearization point.
- Record the decision and align operator/agent guidance.

Out:

- Changing ADR allocation, promotion ordering, merge-queue policy, workflow
  triggers, or App permissions.
- Retrying deterministic check failures without new positive conflict evidence.
- Protecting the controller-owned branch from concurrent privileged external
  mutation.
- Editing `CONTEXT.md` or generated `docs/README.md`.

## Task outline

- [x] Task 1: Implement the typed replacement and publication transaction
  - Files: `xtask/src/pr/promoter.rs`, `xtask/src/pr/snapshot.rs`, and only
    shared helpers required in `xtask/src/git.rs`, `xtask/src/pr/gh.rs`, or
    `xtask/src/test_support.rs`.
  - Contract: raw GitHub values terminate at the existing adapter boundary.
    Typed state carries exact App identity, all-state PR identity,
    `Mergeable`/`MergeStateStatus`, required-check classification, parsed intent
    kind/evidence tuple, generated provenance version/base/replaced head, and
    exact postconditions.
  - Contract: `PromoterGit` separately exposes fresh/exact fetch, sole
    parent/tree/ancestry/local merge-conflict proof, version-selected
    deterministic tree verification, ordinary non-force publication, and
    exact-SHA leased deletion. Ordinary push must never become forceful.
  - Contract: `PromoterPrRead` discovers the latest exact attempt across
    open/closed/merged states and verifies comments through both bot login and
    `performed_via_github_app.client_id`; `PromoterPrWrite` appends immutable
    intent, closes an exact PR, creates, and arms. Existing App permissions
    suffice.
  - Contract: ordered Generate classification is durable intent resume; new
    positive conflict; healthy armed/queued; unarmed incomplete publication;
    closed abort/retirement; validated orphan successor; merged cleanup; empty;
    foreign/ambiguous failure. Dequeue remains exact-head re-arm only.
  - Contract: `Jaunder-Promoter-Version: 1` selects immutable generator/verifier
    semantics. Freshness is checked before push and after PR creation before
    arm; ambiguous mutations are resolved by exact reads; all unpublished
    regeneration loops have a fixed bound.
  - Verification: focused xtask controller and adapter tests cover every
    classification and crash boundary, spoofed/malformed identity, conflict
    precedence, duplicate convergence, deterministic-check behavior,
    pre/post-create main movement, and ambiguous API results. Temp Git
    repositories prove exact-object conflict/ancestry/tree verification,
    leased-delete changed-head refusal, and non-force recreation. Run
    `devtool run -- cargo xtask test-local --manifest-path xtask/Cargo.toml pr::promoter`
    plus focused `git`/`snapshot` tests when they live outside that module.

- [x] Task 2: Record and project the promoter replacement decision
  - Files: new `docs/adr/drafts/replace-conflicted-promoter-attempts.md`,
    `docs/adr/0152-adr-numbering-happens-after-merge.md`,
    `docs/ARCHITECTURE.md`, and `CONTRIBUTING.md`.
  - Contract: the proposed numberless ADR records immutable attempts, dual
    conflict proof, App-authored intents, exact leased retirement,
    generated-tree-verified/versioned provenance, publication linearization,
    generation-only serialization, and the controller-owned-branch trust
    boundary. ADR-0152 receives only a short past-tense annotation; its Decision
    remains immutable.
  - Contract: architecture cites the tracked draft path until promotion.
    Contributor guidance says diagnose and rerun the controller; never manually
    close/delete/rebase/promote. `CONTEXT.md`, `docs/README.md`, workflow
    triggers, and App permissions remain unchanged.
  - Verification: ADR draft/projection links and contributor prose agree with
    the approved spec and implemented state machine; repository static/document
    checks pass through the normal gate.

- [x] Task 3: Update authoritative operator skills and distribute them
  - Files:
    `/home/mdorman/src/agent-configuration/projects/jaunder/.rulesync/skills/jaunder-adr/SKILL.md`
    and `jaunder-ship/SKILL.md` on the current agent-configuration branch.
  - Contract: failed promoter guidance preserves immutable attempts,
    distinguishes visible check failure from recoverable orchestration state,
    directs operators to rerun the controller, and forbids manual
    close/delete/rebase/promote recovery. Generated `.agents`/`.claude` copies
    are distribution outputs, not Jaunder PR inputs.
  - Verification: commit the authoritative files without a `Co-Authored-By`
    trailer and without pushing; run
    `/home/mdorman/src/agent-configuration/bin/refresh-agent-config jaunder` and
    `/home/mdorman/src/agent-configuration/bin/refresh-agent-config --check jaunder`.

- [x] Task 4: Integrate and gate the complete change
  - Contract: all issue acceptance criteria map to observable tests or
    documentation; no obsolete singleton-only Generate path, compatibility
    alias, or manual-recovery guidance remains.
  - Verification: run `devtool run -- cargo xtask check --no-test`, perform
    parallel Standards/Spec and security reviews, then use `jaunder-commit` for
    the staged Jaunder tree. The commit hook owns `precommit`; `jaunder-ship`
    owns pre-push, PR CI, and merge approval.

## Ordering and delegation

- Task 1 and Task 2 may run concurrently because their files do not overlap;
  documentation must use the contracts above rather than inventing
  implementation behavior.
- Task 3 is independent and occurs in the existing agent-configuration checkout;
  do not mix its commit with the Jaunder branch.
- Task 4 starts only after Tasks 1–3 are integrated and their focused evidence
  passes.

## Risk checks

- A stale controller can neither delete nor close a successor within serialized
  controller execution.
- GitHub `UNKNOWN`, queue delay, pending/failed checks, or a healthy external
  close cannot create conflict authorization.
- A durable exact App-authored intent resumes after GitHub mergeability changes
  or the branch is already absent.
- Self-asserted trailers never authorize an orphan: exact deterministic tree
  reconstruction and linked App-authored state are required.
- No main advance between local generation and publication can strand an
  indefinitely stale, unarmed PR.
- Existing real promotion tests continue to prove fresh-main numbering, all
  pending drafts, deterministic slug order, citation/status rewrite, and index
  regeneration.

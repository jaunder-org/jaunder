# Merge-time ADR Promotion Implementation Outline

> Execute with `jaunder-iterate`, delegating task slices through
> `jaunder-dispatch`. This outline exists because the change introduces a
> durable ADR lifecycle boundary, a privileged GitHub App, serialized branch
> mutation, and automatic merge-queue authority.

## Scope

In:

- tracked numberless ADR drafts and tracked-source promotion correctness;
- one promoter-specific xtask controller behind a thin Actions workflow;
- singleton promoter branch/PR ownership, automatic queueing, and fail-closed
  dequeue recovery;
- least-privilege GitHub App provisioning and live workflow proof;
- ADR/process documentation and one-release `adr renumber` deprecation.

Out:

- changing ADR number allocation, index format, or acceptance semantics;
- a general PR automation framework or changes to human `cargo xtask pr land`;
- direct writes or branch-protection bypasses on `main`;
- deleting `adr renumber` in this issue.

## Task outline

- [x] Task 1: File the deferred `adr renumber` removal issue.
  - Contract: P3 Task issue in milestone 3, labeled `tooling` and `dx`, blocked
    by #742, with removal gated on one release exercising serialized promotion.
  - Verification: tracker readback shows Task type, milestone 3, P3 priority,
    labels, blocked-by dependency, and the precise removal condition.

- [x] Task 2: Make tracked drafts and tracked-source promotion correct.
  - Contract: drafts become tracked while remaining outside numbered-ADR
    enumeration; `run_promote` atomically stages the source deletion/destination
    addition, status and path rewrites, and generated index. Existing path-form
    citations remain the sole draft identity. Draft-internal links resolve under
    `doc-links` both before and after the one-directory move.
  - Verification: scratch-repository tests cover tracked rename staging,
    deterministic multi-draft allocation, cross-draft and numbered-ADR links,
    citation rewriting, no-draft rerun, and failure before publication. Preserve
    numbered-ADR gate-invisibility tests. Focused lane:
    `devtool run -- cargo test --manifest-path xtask/Cargo.toml adr::tests`.

- [x] Task 3: Add the promoter controller without changing human PR landing.
  - Contract: an ADR-owned CLI entry invokes a promoter-specific controller in
    `xtask/src/pr/promoter.rs`. It reuses `pr::gh` as the only GitHub subprocess
    boundary and existing snapshot/land/watch factors, but owns injectable Git
    and promoter-PR write traits for stable branch preparation, exact head/base
    singleton lookup, create-or-reuse, push, and queue policy. `PrCommand::Land`
    remains explicit human approval and never gains bot retry behavior.
  - Contract: Task 3 owns the durable identity literals: branch
    `automation/adr-promoter`, PR title `docs(adr): promote pending ADR drafts`,
    body marker `<!-- jaunder-adr-promoter -->`, and deterministic promotion
    commit author/committer `jaunder-adr-promoter[bot]`. Singleton lookup passes
    the branch-only `gh pr list --head` form, then requires parsed repository
    owner, exact head/base, and the marker; title is display text, not identity.
  - Contract: generate from a fresh `main`; local preparation, promotion, and
    diff detection precede queue-policy validation so no draft/diff is a
    successful no-op. A real diff still requires queue and required contexts
    before the deterministic bot commit, push, or GitHub publication. The remote
    promoter head must equal the commit being armed. An existing open promoter
    freezes its head/diff and causes later drafts to wait.
  - Contract: the controller parses `GITHUB_EVENT_PATH` into a typed
    `PromoterEvent`; a dequeue carries action, PR number, exact head ref/SHA,
    and base from the untouched webhook payload. The GitHub read boundary
    resolves that identity to one unique correlated merge-group SHA and
    evaluates required contexts on both it and the unchanged PR head. Re-arm
    only when both context sets exist, match the event identity, are complete,
    and are green. After creation or recovery arms auto-merge, verify the exact
    head has either an auto-merge request or queue membership, using the same
    factors as `pr land`. Absent, ambiguous, mismatched, failed, missing, or
    incomplete evidence terminates without re-arming.
  - Verification: trait-backed controller tests prove deterministic bot commit
    arguments, branch-only lookup plus parsed owner enforcement, singleton
    reuse, no-op before queue reads, pre-publication failure isolation, head
    equality before arm, queued-or-auto-merge verification, bounded green
    dequeue recovery, and no retry for ejection or incomplete evidence. Focused
    lanes:
    `devtool run -- cargo test --manifest-path xtask/Cargo.toml pr::promoter::tests`
    and the affected `pr::land`/`pr::watch` suites.

- [x] Task 4: Wire the serialized Actions workflow and GitHub App boundary.
  - Contract: `.github/workflows/adr-promoter.yml` uses repository `setup-ci`
    and invokes only the promoter xtask command. Triggers are push to `main`,
    manual dispatch, and dequeued pull requests targeting `main`; the workflow
    passes `GITHUB_EVENT_PATH` through unchanged and does not duplicate promoter
    head/marker filtering owned by Task 3. Push/manual generation shares one
    coalescing concurrency group, while dequeue recovery uses a per-PR operation
    group so generation cannot replace a pending recovery. No active operation
    is canceled, and the job logic remains single-sourced.
  - Contract: mint the installation token with
    `actions/create-github-app-token@v3` from repository variable
    `ADR_PROMOTER_CLIENT_ID` and secret `ADR_PROMOTER_APP_PRIVATE_KEY`, then map
    only that step's `token` output to `GH_TOKEN` for the promoter command. The
    App's repository permissions are exactly Actions read, Contents read/write,
    Pull requests read/write, Checks read, Commit statuses read, and mandatory
    Metadata read. Actions read exists only for historical `merge_group`
    workflow-run correlation; the App has no Administration, Actions-write,
    ruleset bypass, or direct-main authority. The built-in `${{ github.token }}`
    is used only where existing `setup-ci` requires it and is never exposed to
    promoter GitHub operations.
  - Contract: use `gh pr merge --auto --merge`; required `pull_request` and
    `merge_group` checks remain unchanged and the queue is the sole writer to
    `main`.
  - Verification: actionlint validates the declarative workflow seam, including
    its operation-specific concurrency key and exact token inputs; controller
    tests prove webhook identity dispatch and unchanged `GITHUB_EVENT_PATH`. The
    `wizard` skill provisions the named App ID/private-key secrets, verifies
    exact App permissions and installation scope, and statically checks that its
    names match the workflow. Live proof covers PR checks, queue entry,
    merge-group checks, queue-only merge, waiting drafts, and safe dequeue
    behavior.

- [x] Task 5: Cut documentation and lifecycle ownership over cleanly.
  - Contract: amend/supersede ADR-0048 through the numberless decision draft;
    preserve ADR-0088; update `docs/ARCHITECTURE.md`, `CONTRIBUTING.md`, ADR
    draft guidance, merge-queue runbook, and the `jaunder-adr`, projection,
    start, and ship skills. Feature shipping must no longer promote or instruct
    collision recovery.
  - Contract: deprecate `cargo xtask adr renumber` and its recovery text without
    deleting the command; link the Task 1 removal issue. Do not hand-edit the
    generated ADR index.
  - Verification: source/docs search finds no live ship-time promotion or
    undeprecated collision-recovery instruction; documentation distinguishes
    healthy proposed-decision lag from failed-promoter lag. Run
    `devtool run -- cargo xtask check --no-test` before the task commit.

## Cross-task contracts

- Task 2 leaves `cargo xtask adr promote` as the deterministic local mutation
  primitive; Task 3 owns remote publication and PR state around it.
- Task 3 exposes one production promoter command; Task 4 contains no duplicate
  Git/PR orchestration shell logic.
- Task 3 owns branch `automation/adr-promoter`, title
  `docs(adr): promote pending ADR drafts`, and marker
  `<!-- jaunder-adr-promoter -->`; Task 4 passes event identity unchanged and
  never re-derives those literals in YAML.
- Human `pr land` policy and promoter auto-merge policy remain separate entry
  points even when they reuse low-level arming and observation code.

## Risk checks

- A partially failed promotion never changes the remote promoter branch or PR.
- A queued promoter's head SHA and generated diff never change.
- Required contexts are evaluated on both the PR head and the correlated
  ephemeral merge-group SHA; PR-head green alone never authorizes retry.
- The App cannot bypass the merge queue or mutate repository administration.
- Drafts remain invisible to numbered-ADR format/index/collision gates while
  becoming visible to `doc-links`.
- Promotion remains the only `proposed` to `accepted` transition.
- Full execution uses each task's focused proof, `jaunder-commit`'s precommit
  gate per commit, and the repository validation ladder before shipping.

# Design: history rebuild into a fresh, legible repository

**Status:** proposed · **Date:** 2026-06-22 · **Author:** Michael Alan Dorman (with Claude)

Supporting analysis (commit counts, churn, decomposition, extractability) lives in the working
notes `HISTORY-REWRITE-SURVEY.md`; this spec records the *decided* plan.

## 1. Goal & win condition

A git history that can be **understood over time**. Concretely: `git log --first-parent` should
read as a deliverable-level changelog — a reader grasps how the project progressed and when
features landed without reverse-engineering 700+ commits. The GitHub *PR artifacts* are not a goal;
the merge **shape** is. Everything below serves that.

## 2. Target shape

A **flat deliverable spine**: ~50 top-level `--no-ff` merges, one per coherent deliverable, each
carrying an **authored narrative message**. Detail commits sit behind each merge. `git log
--first-parent` is then the changelog; those ~50 merge messages *are* its entries.

Deliverables are anchored to the repo's own plan/spec doc catalog (~45 docs), which is the de-facto
deliverable manifest.

## 3. Engine

Two phases over a `filter-repo` pre-pass. **No `--rebase-merges`** — its purpose is recreating
*existing* merges, and we keep none; conflating cleanup with merge-building is what made it look
necessary. Separating the two is both more correct and simpler.

**Pre-pass (`filter-repo`).** Rebase cannot strip paths, so first: excise `.beads/` and
`HISTORY-REWRITE-SURVEY.md` from all history, prune the ~40 now-empty beads-only commits, and scan
for large blobs (flag, never silently drop). Everything downstream sees beads-free commits.

**Phase A — clean linear history**, split into two sub-steps to **bound conflict blast-radius**
(an early conflict in a single giant replay could otherwise ripple onto every following commit):

- **A1 — pure linearization.** Replay all commits in topological order with *no* reordering or
  squashing — only drop beads-only remnants and normalize messages. Because all 40 original merges
  are **trivial** (the branches never touched conflicting lines), this global pass is expected to be
  near-conflict-free, and it **converges to the baseline** (tip tree == frozen baseline). Output:
  `L0`, a flattened-but-faithful line.
- **A2 — localized cleanup.** Apply the conflict-prone transforms as small, *independent, regional*
  rebases on top of `L0`: collapse do-then-redo (adjacent-commit-local) and reorder only the
  **file-disjoint ("clean")** detours from the extractability analysis (entangled ones stay put).
  Each edit's blast radius is its own region — a conflict can't cascade across the whole history.

`exec <gate>` after each pick (both sub-steps) → the per-commit green gate; a non-zero exec **halts**
the rebase exactly there, you fix, `git rebase --continue`. Author dates preserved; committer dates
= replay time.

Output: a clean, green, **linear** history `L` (= `L0` + the A2 edits) whose tip tree equals the
frozen baseline (lossless).

**Review gate — an explicit, mandatory pause between A and B.** Phase A ends by tagging the linear
history (`linear-clean`) and proving its tip tree equals the frozen baseline (zero accidental
drift). The process then **stops** for author review — Phase B does not begin automatically. You
inspect `L` directly (the linear log, the normalized messages, the overall story) and may:
- do additional `rebase -i` cleanup of your own (further reorder/squash/reword), and/or
- make **intentional content fixes** you spot along the way.

Intentional edits are expected and welcome; they surface as an explicit `baseline..L'` content diff
so "intended change" is never confused with "accidental drift." Phase B runs **only on your
explicit go**, over the reviewed line `L'`.

**Phase B — synthesize the merge spine over `L'`.** Walk `L`; for each deliverable (a contiguous
run) build a `--no-ff` merge, so `git log --first-parent` becomes the ~50-entry changelog. Because
each deliverable is a linear extension of the prior merge point, every merge is fast-forward-clean
— so it is built with `git commit-tree` plumbing: `merge = (deliverable-tip tree, parents
[previous-merge, deliverable-tip], authored message)`. This is **conflict-free, keeps Phase A's
green commit SHAs intact** (no re-checking), and produces byte-for-byte what `git merge --no-ff`
would. Per-merge gates (tests incl. PG; content-aware e2e, §6) run at each synthesized merge tip.

**Executed in checkpointed segments, by era** — tag a verified checkpoint after each so a mistake
costs one segment, not the whole run.

**All conflict risk lives in Phase A's reorder/squash picks** — exactly where a human belongs.
Phase B is conflict-free by construction.

## 4. Cleaning transforms

1. **Excise beads entirely.** Paths (`.beads/`, incl. its 5 git hooks) + textual references in
   *tooling/config/guidance* files only (`.gitignore`, `flake.nix`, `AGENTS.md`, `CONTRIBUTING.md`,
   `CLAUDE.md`). Plan/spec **prose** mentioning bead IDs is left as honest historical record.
2. **Strip all `Co-authored-by` trailers** (395+ commits, all Claude models, no humans). Omitted
   going forward.
3. **Rewrite commit messages (full message, not just subjects), tiered.** Subjects → Conventional
   Commits `type(scope): …` (scope vocabulary: `common · server · storage · web · e2e · build ·
   docs · deps`). Bodies are brought up to the standard the *later* commits already set:
   - **already-informative messages** (mostly recent) → keep the body, normalize the subject only.
     Don't gratuitously rewrite good messages — they hold authentic at-the-time context that a
     post-hoc reconstruction from the diff would lose.
   - **rough messages** (title-only, uninformative, or bare "see doc X" pointers — common early on)
     → author a proper body from the diff, **inlining the relevant context from the now-resurrected
     planning docs** rather than pointing at them, so the message is self-contained.

   Squashed do-then-redo commits get a single *composed* message for their net change. The 49
   beads-referencing subjects are re-derived from diffs; **no `chore(beads):` appears anywhere**.

   **Honesty guardrail:** a rewritten body states only what is verifiable from the diff and the
   referenced docs — accurate *what*, no invented *why*. Legible, not fictional.

   This is the **largest judgment workload** in the project (~670 messages): produced out-of-band as
   a reviewable **old→new mapping**, pre-baked into the Phase-A todo, and shown in full context at
   the review gate (§3).
4. **Documentation lifecycle — `plans/`+`specs/` are ephemeral.** One rule across the rebuilt
   history: a plan/spec doc lives in `docs/superpowers/plans|specs/` while its deliverable is in
   development, then **retires to `docs/archive/`** at that deliverable's completion (its last
   commit, from the §5 attribution). So at *any* commit, `plans/`+`specs/` show only in-flight work
   and `archive/` holds finished work; at HEAD only genuinely in-flight docs (e.g. the emacs plan)
   remain unarchived. The **30 dropped docs** are a special case — completed docs that were
   *deleted* rather than archived; they are resurrected into the lifecycle and land in `archive/`
   (verified all truly dropped: 0 relocated, 0 absorbed). This deliberately reorganizes the final
   `docs/` tree; the §6 code-lossless invariant is unaffected (it is code-only).
5. **ADRs are permanent — exempt from the lifecycle.** `docs/decisions/` → `docs/adr/` retroactively
   (consistent path from first appearance; log4brains/adr-tools-compatible), and they are **never
   archived**.
6. **Collapse do-then-redo** — the high intra-branch rework (PR #28 `backup-module-reorg` is the
   clear case; spot-check M2/M5).
7. **Reorder spliced detours** into their own deliverables where cleanly extractable (per the
   extractability map); entangled detours stay merged with their host.

## 5. Decomposition & commit-homing

~50 deliverables from the plan/spec catalog. The ~270 cross-cutting "generic" commits (38%, no
feature keyword: coverage, infra/CI/xtask, docs, clippy, refactors, dep bumps) are homed
**hybrid**:

- a deliverable's *own* tests/coverage/docs ride **with** it (self-contained features);
- the genuinely cross-cutting remainder batches into a few **periodic maintenance merges**
  (`build: xtask/CI`, `chore: coverage tooling`, `docs`, `deps`) — mirroring the project's actual
  rhythm (it already shipped `clippy-pedantic-fixes`, `mutants`, `code-motion`, `reorg`,
  `maint-setup` as standalone maintenance PRs).

## 6. Verification strategy

Run the **gates directly** (`cargo clippy`, `cargo fmt --check`, `nextest` incl. Postgres, the e2e
suite) — not the `xtask` wrapper. Each gate runs against **that commit's own tree** (its
contemporaneous `clippy.toml`/lints), so we are *not* retrofitting later strictness onto earlier
commits — a commit only satisfies the rules that existed when it was authored.

| Scope | Gate |
|---|---|
| **Every commit** | compiles + clippy-clean + fmt-clean (`check --no-test` equiv), via `exec`, fixed by amend/reorder/squash |
| **Every merge** | + tests pass incl. PostgreSQL (`check` equiv) |
| **Merges touching `web/`, `end2end/`, or server HTTP** | + full e2e (content-aware; pure backend/storage/tooling merges skip e2e) |
| **Final tip only** | full `validate` + regenerate coverage / CRAP / baseline manifests |

**Coverage *regression* checking is suspended for the duration** — the baseline is itself in flux
during the rewrite, so per-merge regression is meaningless; the manifests are a final-tip concern.

**Baseline & freeze:** `main` is tagged `pre-rebuild` at a chosen commit and feature work **pauses
(or branches off the tag)** until cutover. "Current `main`" throughout this spec means that frozen
baseline, so the lossless target is well-defined even though day-to-day work could otherwise move
`main`. Any work landed after the freeze is replayed onto the rebuilt history (or re-cut as its own
deliverable) at cutover.

**Invariant (code-scoped):** the **code** is lossless — every path *outside* `docs/`, `.beads/`, and
`HISTORY-REWRITE-SURVEY.md` is byte-identical to the frozen `pre-rebuild` baseline at the
corresponding point. `docs/` and the excised paths change only by the **deliberate** §4 transforms
(beads excised, ADRs relocated, docs run through the lifecycle). The sole *unplanned*-content
exception is the author's **intentional edits at the §3 review gate**, surfaced as an explicit code
diff vs baseline; the final repository tree is `L'`. Automated green-fixes must come from
**reordering/squashing existing work**, never net-new code — a commit that can only go green with
code existing *nowhere* in history signals a latent gap, to be surfaced rather than papered over
(deliberate human fixes at the review gate are the exception).

## 7. Cutover (Option 2: delete & recreate)

PRs cannot be deleted on GitHub — only the repo can. Since the merge spine is built from **local**
`--no-ff` merges (no PRs created), a fresh repo naturally has an empty Pull-requests tab and zero
orphaned PRs.

**Sequence (destructive steps last, every prior step verified):**

1. Dump settings via `gh api` → manifest.
2. `git bundle create --all` the existing repo; **verify by cloning from the bundle** and diffing.
3. Push the rehearsal-verified rebuilt history (§8B output).
4. Delete `jaunder-org/jaunder`; recreate `jaunder` at the same URL; push the rebuilt history.
5. Restore settings from the manifest via `gh api`.
6. Manually re-enter the write-only items (the dump lists their names): Actions/Dependabot/env
   **secret values**, webhook secrets, deploy private keys.
7. Migrate the surviving open backlog → GitHub issues (§9).

**Settings auto-migrated:** core settings, branch protection / rulesets, labels, webhooks (minus
secret), Actions variables + permissions, environments, collaborators/teams, autolinks, custom
properties, security flags.
**Lost / manual:** secret *values* (re-entered), deploy private keys (re-added), stars/watchers/forks
(unrecoverable). Deleting the repo also **permanently erases all issue/PR discussion threads** —
intended for the merged PRs; accepted for closed issues (open work is migrated).

## 8. Rehearsal plan (all non-destructive; real repo & GitHub untouched until proven)

- **0. Linearization conflict measurement (do this first, it's a go/no-go).** On a clone, run A1
  alone (pure linearization, no cleanup) and **count the actual conflicts**. The trivial-merges
  result predicts near-zero; if that holds, the cascade worry is empirically dismissed before we
  invest in anything else. A surprisingly high count is a signal to rethink scope *now*, cheaply,
  rather than mid-rebuild.
  **Result (measured 2026-06-22): 0 conflicts across all 716 non-merge commits; final linear tree ==
  HEAD tree (lossless verified).** A1 cascade risk retired — any real-run conflict can come only from
  A2's localized reorder/squash, never the linearization.
- **A. Machinery smoke test** — full pipeline on a *slice* (last era / a few deliverables) on a
  scratch clone. Validates todo generation, Phase-A `exec` gates halting, Phase-B spine synthesis,
  message rewrite, tree-equality. Fast.
- **B. Full local dress rehearsal** — entire pipeline on a full clone, end to end, producing the
  complete rebuilt history. Verify final-tree == HEAD, all ~50 merges, greenness at the §6 tiers.
  This output is also the **readable preview** — confirm `--first-parent` tells the intended story
  *before* any cutover. This is the go/no-go artifact.
- **C. Settings round-trip** — dump real settings → throwaway repo → restore → diff → delete
  throwaway. Proves the migration without touching the real repo.

Go live only after A + B + C pass.

## 9. Open-issue backlog migration

26 open beads (304 closed = history, vanish with `.beads/`). Triage: visibility-related items
already resolved (visibility merged); stale items revisited; genuine backlog → GitHub issues in the
new repo via `gh issue create` (title/body, `issue_type`→label, `priority`→`P1`–`P4`, epics→
milestones, deps→`blocked by #N`). Design context links to `docs/`, not duplicated.

## 10. Risks, invariants, non-goals

**Risks & mitigations:**
- Long-running rebase fragility → segment + checkpoint tags.
- Irreversible repo delete → verified `git bundle` + rehearsal C before any delete.
- Clippy strictness over time → gates use each commit's *contemporaneous* config (achievable bar).
- Cleanup-pick conflicts → rebase halts; resolved with a human in the loop.

**Non-goals:** preserving old PR/issue artifacts; preserving merge timestamps; coverage-regression
continuity during the rewrite; GPG signing (originals are unsigned — unchanged).

**Deferred to the implementation plan:** the exact ordered commit→deliverable assignment (the
per-commit attribution behind §5), the segment boundaries, and the precise per-deliverable merge
messages.

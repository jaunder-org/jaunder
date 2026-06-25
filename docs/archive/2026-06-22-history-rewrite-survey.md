# Git history rewrite — Phase 1 survey

> **Status: COMPLETE** — phase-1 scratch survey for the git history rewrite, which
> was executed and verified (see the `2026-06-22-history-rebuild-*` entries in this
> directory). Originally self-labeled "delete when done"; archived instead to
> preserve the record. Archived under issue #39.

Working scratch document. Read-only analysis; nothing has been mutated. Delete when done.

## 0. Chosen approach (decided)

**Linearize → clean → re-cut into fresh PRs.** Do not preserve the existing merges.

1. **Linearize** the existing history — flatten all 715 non-merge commits into a single line in
   topological order; drop all 40 merge commits.
2. **Clean on the line** (far easier than on a DAG): excise `.beads/` (§6), normalize commit
   messages and strip all `Co-authored-by` trailers (§3), resurrect the dropped planning docs (§7), collapse do-then-redo (PR #28 et al,
   §4), reorder the spliced detours (§8). Verify **final tree == current HEAD tree** after every
   step — the invariant that proves each rewrite is content-lossless.
3. **Re-cut & replay** into a fresh repo: walk the clean line, spin off a branch per coherent
   deliverable (§8 decomposition), open a real GitHub PR, merge, rebase the remainder, repeat —
   minting *new* merges with real narratives at *good* boundaries. Old repo kept as archive; old
   PR numbers abandoned.

**Why drop the merges:** all 40 are bare `Merge pull request #N` with empty bodies (§2) marking the
*bad* decomposition (§8) — preserving them preserves nothing while locking in what we want to fix.
Dropping them also removes the single most conflict-prone part of git surgery (`rebase
--rebase-merges`). The replay regenerates better merges, so nothing is lost.

**De-risk (verified):** all 40 merges are **trivial** — each recorded tree exactly equals a clean
auto-merge of its parents (0 conflicts, 0 evil merges, git 2.54). So linearization is **provably
lossless**: the flattened line reproduces HEAD's tree exactly, with no merge resolutions to carry.

Remaining conflict risk lives only in step 3 (re-cutting interleaved deliverables, §8
extractability) — bounded, per-PR, and optional (entangled deliverables can stay merged).

## 0.1. Update — 2026-06-22 (visibility merged; +3 PRs)

`visibility` is merged; `main` is green. Three PRs landed since the original survey snapshot below,
all re-measured here. **Re-verified: all 40 merges remain trivial** (0 conflicts / evil merges) —
linearization is still provably lossless. The §1–§9 numbers below are the original snapshot; deltas:

| Metric | original survey | now |
|---|---|---|
| total commits | 735 | **755** |
| merges | 37 | **40** |
| non-merge | 698 | **715** |
| `Co-authored-by` trailers | 395 | **423** (still 100% Claude, no humans) |
| conventional conformance | 52% | 53% |

**New PRs — all clean single-deliverable, i.e. they already model the §8 target:**
- **#38 `ci-test-speed` (17c)** — consolidates the integration tests into per-area binaries, plus
  the *arguable* `cheap-kdf` test feature (cheap Argon2 params in test builds, fenced by a release
  tripwire + a fail-closed binary guard). New deliverable threads: `test-consolidation`, `cheap-kdf`.
- **#40 `visibility` (30c)** — Content Visibility Layer A/C; **rebased onto post-#38 main** (hence
  `test: adapt visibility suite to main's reorganized test layout`). Clean single deliverable.
- **#41 `emacs` (1c)** — `Plan for emacs front-end` (the one new non-conforming subject; a planning
  doc only, no code yet). Add to the deliverable catalog as a forward thread.
  (PR #39 is absent — a numbering gap, closed/unmerged; nothing to carry.)

**This doc got committed accidentally** (`b2d5417`, `64c9620`) — but both are `chore(beads)` commits
touching *only* `.beads/` + `HISTORY-REWRITE-SURVEY.md`, so once both paths are on the excise list
(§0 step 2, §6) they become empty and **prune automatically**. The accident self-cleans; no special
handling needed.

## 1. Shape of the history

| Metric                              | Value                             |
|-------------------------------------|-----------------------------------|
| Total commits (reachable from HEAD) | 735                               |
| Merge commits                       | 37                                |
| Non-merge commits                   | 698                               |
| Distinct authors                    | 1 (Michael Alan Dorman)           |
| Date span                           | 2026-03-28 → 2026-06-21           |
| First-parent spine length           | 68 (37 merges + 31 trunk commits) |

The trunk is clean: **every historical change landed through one of 37 GitHub PRs.** The 31
non-merge commits on the first-parent spine are *not* loose history — they are the current
in-progress `visibility` branch (well-formed `feat(...)`/`chore(beads)` commits stacked on top
of PR #37) plus the root commit. So the DAG is a tidy "37 PRs off `main`" picture, exactly the
topology you want to preserve.

## 2. The PR spine (merge = PR integration)

Every merge is the bare GitHub default — `Merge pull request #N from jaunder-org/<branch>` with
**no body**. This is the "garbage merge messages" problem: 37/37 carry zero narrative about what
the PR enabled. Branch names are meaningful, which is what makes good merge narratives writable.

```
PR  date        #commits  branch
#37 2026-06-20    78  testing-coverage-orchestration
#36 2026-06-18    14  metrics
#35 2026-06-18    31  epic            (branch "epic" reused — also #34)
#34 2026-06-15    22  epic
#33 2026-06-14    18  testing
#32 2026-06-12    21  simplify
#31 2026-06-02     8  maint-setup
#30 2026-06-02    24  atompub
#29 2026-05-29    32  M8
#28 2026-05-25    17  backup-module-reorg
#27 2026-05-25    14  web-module-reorg
#26 2026-05-24     1  web-restructure
#25 2026-05-23    45  test-coverage
#24 2026-05-21     4  media-manager
#23 2026-05-21     5  mutants
#22 2026-05-20     5  code-motion
#21 2026-05-20    11  storage-split
#20 2026-05-19     6  misc-fix
#19 2026-05-19     3  reorg
#18 2026-05-18     1  coverage
#17 2026-05-16    18  tags
#16 2026-05-15     3  docs
#15 2026-05-13    24  aesthetics
#14 2026-05-12    35  feeds
#13 2026-05-02     2  verify-optimization
#12 2026-05-01     9  clippy-pedantic-fixes
#11 2026-05-01     9  backup
#10 2026-04-28    38  ui
#9  2026-04-21    61  M5
#8  2026-04-10    18  M4
#7  2026-04-09     3  misc
#6  2026-04-09    15  M3
#5  2026-04-06    39  M2
#4  2026-04-01     2  CI       (branch "CI" reused — also #3)
#3  2026-04-01     1  CI
#2  2026-03-31    14  M1
#1  2026-03-30    16  M0
```

## 3. Commit-message style audit

|                                             | count | % of non-merge |
|---------------------------------------------|-------|----------------|
| Conventional-commit form (`type(scope): …`) | 363   | 52%            |
| Non-conforming                              | 335   | 48%            |

Type distribution among the conforming half:
`feat:105  docs:58  refactor:57  test:47  fix:33  chore:32  build:15  ci:7  style:7  perf:2`

**The non-conforming half is salvageable, not garbage.** It follows older, recognizable
conventions rather than being random:

- 175 start with a capitalized imperative verb/scope; 52 lowercase; 108 "other" (mostly the
  early milestone-numbered style, e.g. `M0.0.1: Split PostStorage into …`, `M7.3+5: …`).
- Leading-word clusters (top): `close`(33) & `task`(8) = beads task-tracking commits
  (`Close jaunder-gb3: …`); `add`(29), `update`(10), `move`(6), `make`(4), `remove`(4),
  `improve`(5) = plain imperative verbs; `ui`(16) & `observability`(8) & `web` = scope-first
  style; the `M*` numbered subjects = milestone task IDs.

### Proposed normalization rules (mechanical, with a few judgment calls)

| Old shape                                       | → New                                                                           |
|-------------------------------------------------|---------------------------------------------------------------------------------|
| `Add <x>` / `Implement <x>` (new capability)    | `feat(<scope>): <x>`                                                            |
| `Move <x>` / `Refactor <x>` / `Consolidate <x>` | `refactor(<scope>): <x>`                                                        |
| `Update/Improve/Make <x>`                       | `chore(<scope>): …` or `refactor(...)` per content                              |
| `<scope>: <text>` (e.g. `ui: …`, `flake: …`)    | `<type>(<scope>): <text>` — infer type from diff                                |
| `M0.0.1: <text>` milestone-numbered             | `<type>(<scope>): <text>` (milestone id → body; this is a *doc* ref, not beads) |
| Typos (`Consoldate`)                            | fix spelling while normalizing                                                  |

Scope is inferred from the touched paths (`server/` → server, `web/src/` → web,
`flake.nix`/`xtask` → build, `docs/` → docs).

### Beads-referencing subjects (interacts with §6 excision)

85 commit subjects reference beads (`Close jaunder-xxx:`, `Task N:`, `chore(beads):`, or "beads"
in prose). Handling is driven by §6's path-strip + prune, not by a message map:

| Population | count | handling |
|---|---|---|
| beads-only commits | 36 | **pruned** by `--prune-empty` — no message to write |
| survive with code (mostly `Close jaunder-xxx:` test-coverage commits) | 49 | strip the `Close jaunder-xxx:` / `Task N:` framing; derive `type(scope):` from the diff, keep the descriptive remainder. e.g. `Close jaunder-gb3: backup/mod.rs size-mismatch and missing-parent test cover` → `test(storage): cover backup size-mismatch and missing-parent` |
| survivors that mention "beads" in *prose* (≈6) | — | reword to drop the beads clause (e.g. `chore: spec note + sync beads task state` → `docs(specs): disk-footprint note`) |

There is **no `chore(beads):` output anywhere** in the rewritten history — the scope disappears
entirely (25 existing `(beads)`-scoped commits are either pruned or re-scoped).

### Strip Co-authored-by trailers (all commits)

**395 of 719 commits** carry a `Co-authored-by:` trailer; **all are Claude models**
(Opus 4.8/4.7/4.6 · Sonnet 4.6 · Haiku 4.5, all `noreply@anthropic.com`) — **no human co-authors**,
no "Generated with Claude Code" lines, no 🤖. A blanket strip is safe. The message callback removes
each `Co-authored-by:` line **and its preceding blank line**, applied to every commit (independent
of the normalization rules above). Going forward, new jaunder commits omit the trailer entirely.

This is the bulk of Phase 3a; I can generate the full old→new mapping (for the ~670 commits that
survive the prune) for your review once you approve the rules.

## 4. Do-then-redo: much less than feared

Churn analysis (gross line-churn ÷ net line-churn, per PR branch ≥4 commits). High ratio = work
written then rewritten/undone *within* the branch.

- **Median ratio ≈ 1.1×.** Almost no intra-branch thrash. Work accumulates; it is not
  written-then-rewritten. The big repeated-file counts (`.beads/issues.jsonl` touched 37× in
  one PR) are just the task-tracker file rewritten every commit — noise, not rework.
- **Only one real outlier: PR #28 `backup-module-reorg` at 1.9×** (~800 lines added then removed
  within the branch). That is the single branch where collapsing commits would clearly help.
- The big milestone PRs (M2/M4/M5) carry some absolute rework but at ~1.1× of large feature work
  — normal, not worth restructuring.

### The "redo" you remember is mostly *across* PRs, and should NOT be collapsed

There is a deliberate reorganization cadence — PRs `reorg`(#19), `code-motion`(#22),
`storage-split`(#21), `web-module-reorg`(#27), `backup-module-reorg`(#28), `simplify`(#32). That
is "build then reorganize," but each reorganization was its own PR. Collapsing them would violate
your "preserve the PRs" constraint, and they tell a legitimate story (the project's refactoring
rhythm). Recommendation: leave the cross-PR reorg cadence intact; only consider intra-branch
collapse for PR #28.

## 5. What this means for the plan

The cleanup is **overwhelmingly a message job, not a restructure job** — which is the cheap, safe
pass. Concretely:

- **Pass 1 (low risk, high value):** rewrite all 37 merge messages with real narratives + normalize
  the 335 non-conforming commit subjects to conventional-commits. Topology untouched, end-state
  tree provably identical. `git filter-repo` with a hash-keyed message callback. I can drive ~all
  of this; you review the 698-row mapping + 37 merge narratives before it runs.
- **Pass 2 (optional, higher risk):** intra-branch commit collapse — and the data says this is
  worth doing for **PR #28 only**, maybe spot-checking M2/M5. This is `rebase --rebase-merges`
  territory and the only place merges get re-performed.

This sharply changes the earlier risk picture: the scary part (restructuring across a DAG of
merges) is needed in ~1 place, not pervasively. The replay-into-a-new-repo idea remains viable but
is now clearly the expensive 10% — Pass 1 alone gets you a clean, consistent, well-documented
history while preserving every branch and merge.

## 6. Excise beads from all of history ("as if it never existed")

Decision made: retire beads. Project tracking moves to **GitHub issues** (lightweight, no repo
artifact, auto-links to PRs); durable planning/detail stays in **in-repo markdown** under `docs/`.
The rewrite removes the `.beads/` footprint from every commit.

**Footprint:** 10 distinct paths ever under `.beads/` (`issues.jsonl`, `config.yaml`,
`metadata.json`, `README.md`, `.gitignore`, and 5 `hooks/*`); touched by 243 commits.

**Mechanics:** `git filter-repo --path .beads/ --invert-paths --prune-empty`, folded into the same
pass as the message rewrite. Effect on the 706 non-merge commits reachable from HEAD:

|                                             | count | result                                         |
|---------------------------------------------|-------|------------------------------------------------|
| touch **only** `.beads/` (pure bookkeeping) | 40    | become empty → **pruned**, vanish from history |
| touch `.beads/` **+ code**                  | 187   | kept, beads part stripped from the diff        |
| never touch `.beads/`                       | 479   | untouched                                      |

So excising beads is a *net narrative win*: 40 `close .X / open .Y` noise commits disappear and 187
diffs shrink. The 1.0.4 memory-order investigation is now moot — the whole directory goes.

**Textual references (scope decision):** 23 tracked files at HEAD still mention beads/`bd`. They
split into two kinds:
- **Tooling/config/guidance** — `.gitignore`, `AGENTS.md`, `CONTRIBUTING.md`, `CLAUDE.md`,
  `flake.nix`. These describe the *project setup* and should be scrubbed so the repo looks
  beads-free. (Recommended.)
- **Plan/spec prose** — ~16 `docs/superpowers/plans|specs/*.md` that mention bead IDs
  (`jaunder-gb3`) as historical record of what was tracked when. Rewriting these across history is
  high-effort / low-value and arguably falsifies the record. (Recommend: **leave as-is**.)

→ Decide: scrub tooling-only (recommended) vs full prose scrub of every bead-ID mention.

**Open backlog migration.** 26 of 330 issues are open (25 open + 1 in-progress); the other 304 are
closed → pure history, they vanish with `.beads/` and need no action. Triage the 26 into:
- *Resolves with `visibility`* — the `jaunder-i3il` Content-Visibility epic + `i3il.12`/`i3il.13`
  sub-tasks, and the P1 visibility bugs `yhm7`/`ow6h`; finishing that branch closes/handles them.
  Don't migrate.
- *Stale, revisit* — the 38-day UI items (`80z`,`ka4`,`urw`,`h1u`) and 19-day maintainability set
  (`r3nt`,`ic9`,`dsd3`,`o90k`); keep only what's still real.
- *Genuine backlog → migrate to GitHub issues* — the rest (`kq8w` epic + `.22/.24/.25`, `h3g6`,
  `4yjr`, `b2i1`, `5d7n`, `cljg`, …).

Migration (runs when the fresh repo exists): `gh issue create` per surviving issue —
`title`→title, `description`→body, `issue_type`→label, `priority`→`P1`–`P4` label; epics →
milestones (or tracking issue + child checklist); the 6 dependency-bearing issues → `blocked by #N`.
Design context stays in the resurrected `docs/`; issues link the doc rather than duplicating it.

## 7. Resurrect the dropped planning corpus

The "moment we git-deleted planning docs" is two deliberate-reorg commits on 2026-05-15:
- `368a35fa` *"docs: reorganize documentation and adopt MADR-style ADRs"* (PR #16 `docs`) — removed
  ~44 paths: the **M0–M7 milestone docs** + the early `superpowers/plans` & `specs` corpus.
- `e90fee6` *"docs: redistribute observability.md into ADRs and primary docs"*.

**Manifest (generated) — all of it was genuinely dropped.** With rename detection on, the reorg
shows **0 renames and 0 content-absorption**: the MADR reorg *authored new ADRs and deleted the
old docs* rather than migrating them. Every dropped file's best similarity to any current `docs/`
file is ≤0.24 (M5 milestone, the closest), so nothing survives at HEAD to duplicate. Resurrect set
is unambiguous: **all 30 files, ~506 KB** — the M0–M7 milestone docs, the early
`superpowers/plans/2026-04-*…2026-05-04` corpus, their `specs/` designs, plus `observability.md`,
`ui-import.md`. Largest: `2026-04-26-post-ui-consistency.md` (72 KB), `2026-05-02-m7-media-handling.md`
(72 KB), `2026-05-01-clippy-pedantic-fixes.md` (34 KB).

**Where they land:** back under `docs/` (likely `docs/milestones/` + `docs/plans/` or a
`docs/archive/` if you'd rather mark them historical).

**Execution differs by delivery model:**
- *Replay into a fresh repo* → simply **don't replay the deletion commit**; the docs live
  continuously from their original introduction. Cleanest — fits the "never deleted" story.
- *Force-push in place* → restore the truly-dropped set in one tip commit
  (`docs: restore planning corpus dropped in the MADR reorg`). Honest and simple; the docs blink
  out at PR #16 and return at the tip.

## 8. Proposed PR re-decomposition (analysis)

**Method.** Existing PR boundaries aren't the unit of "a subject of work" — a single feature
legitimately spans server+web+e2e+docs, so file-area mix is a red herring. The real signal is the
*deliverable*. Two anchors: (a) keyword-clustering all 667 work commits into deliverable threads,
and (b) the repo's **own plan/spec docs**, which turn out to be a deliverable catalog of ~45 units
(27 surviving at HEAD + ~18 resurrected). One plan/spec doc ≈ one coherent deliverable ≈ one ideal
PR. The 37 actual PRs map to that catalog very unevenly.

**Two independent patterns (only the first is a problem):**

1. **Oversized PRs bundle many distinct deliverables** — the split candidates. Anchored to plan docs:

| PR (size)                                 | distinct deliverables it bundles (≈ plan docs)                                                                                                                           | suggested # PRs |
|-------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------|
| #37 `testing-coverage-orchestration` (78) | xtask-foundation · coverage-postprocessing-engine · http-layer-rstest · rstest-backend-parametrization · xtask-command-model · (testing-coverage-orchestration umbrella) | **5–6**         |
| #5 `M2` (39)                              | invite-storage · create-user-with-invite · registration-policy · user-commands · account-pages                                                                           | **4–5**         |
| #9 `M5` (61)                              | e2e-harness · hydration-fix · post-ui · metrics/perf groundwork · storage                                                                                                | **4–5**         |
| #10 `ui` (38)                             | ui-polish · post-ui-consistency · titleless-posts · post-composer-consolidation                                                                                          | **3–4**         |
| #32 `simplify` (21)                       | split-render-module · orgize-alpha-migration · session-storage-dedup-dialect                                                                                             | **3**           |
| #14 `feeds` (35)                          | feeds · m7-media-handling · media-routing-collision-fix (media work landed here)                                                                                         | **2–3**         |
| #29 `M8` (32)                             | m8-feeds · pg-bootstrap-to-storage · storage                                                                                                                             | **2–3**         |
| #30 `atompub` (24)                        | atompub-interface · (+ accounts/post-ui residue)                                                                                                                         | **2**           |

2. **Cross-cutting threads spread thin across many PRs** — these should *stay* distributed, NOT be
   consolidated. They're continuous concerns, not deliverables:
   - `coverage` (65 commits across **19** PRs), `storage` layer (77/19), `accounts` (34/17),
     `email` (16/12), `clippy` (11/8), `nix`/`flake` (25/12).
   - Forcing "all coverage into one PR" would be wrong — coverage rides along with each feature.
     The clean exception is a few genuinely-periodic passes (`clippy-pedantic-fixes`,
     `cargo-mutants-coverage-analysis`) that already are, and should stay, their own maintenance PRs.

**Clean deliverables already well-contained (leave alone):** `xtask` is 27 commits all in #37 (so
it's a clean *slice* of an oversized PR, easy to extract); `orgize` (5, all #32), `tags` (mostly
#17), `backup` (mostly #11+#28), `metrics` (mostly #36). These are the model the rest should match.

**What the rebuilt set would look like:** ~**50–55 single-deliverable PRs**, each ≈ one plan/spec
doc, grouped under the existing milestone/epic names (M0–M8, feeds, atompub, testing-coverage) as
GitHub *milestones* or epic labels rather than as mega-PRs. Net: same topology philosophy
(branches + merges preserved), but the merge points become meaningful per-deliverable boundaries
instead of per-milestone dumps.

**Caveats (important):**
- This only applies in the **replay-into-fresh-repo** model (§5) — you're authoring new PR
  boundaries. In force-push-in-place you keep the 37 PRs as-is.
- **Interleaving = reordering risk.** Within a milestone PR the deliverables were often developed
  interleaved in time; cleanly separating them means reordering commits, which can introduce
  conflicts (same caveat as the §pass-2 restructure, now at larger scale). Some deliverables won't
  separate cleanly and are better left merged.
- Exact commit→deliverable assignment needs a per-commit pass (the keyword threads are ~85%
  confident; the residual generic/infra commits — 38% have no feature keyword — need manual
  bucketing). I can produce that assignment as the next artifact if you pursue this.

### Detour-extractability (validates the "avoided branch-spin" theory)

Per-commit analysis confirms the oversized PRs are **a main thread with contiguous detours spliced
in** — the signature of "noticed a problem, fixed it inline instead of spinning a branch." For each
detour run we tested whether it shares *non-sink* files with earlier same-branch commits
(sink = a file ≥20% of branch commits append to: `mod.rs`, shared test files, manifests,
`.coverage-baseline`). No real shared file ⇒ the detour lifts out to a base PR and the main thread
rebases on top (the branch-off-main move, applied retroactively).

| Verdict                  | PRs                                                                                                                                                                                                                                               |
|--------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Cleanly splittable**   | **#32 simplify** is the standout — render / orgize / storage-dialect / atompub detours all lift cleanly (matches its 3-plan-doc split exactly). Also clean chunks in #8, #10 (a 14-commit run), #25, and #37 (media, email, one coverage detour). |
| **Partially splittable** | #37, #30, #5 — some detours lift, others are bound to earlier work.                                                                                                                                                                               |
| **Genuinely entangled**  | **#9 M5** (worst — 8 entangled detours; the metrics/post-ui/hydration work genuinely co-evolved), #14 feeds, #34 epic, #15 aesthetics. Better left merged or stacked.                                                                             |

**Calibration (important):** this is file-level, so it **over**-states entanglement. Many
"entangled" verdicts hinge on shared *append-only* files — `server/tests/storage.rs`,
`web/.../tests/web_posts.rs`, `storage/{sqlite,postgres}.rs` — where two deliverables touch the
same file without one *depending* on the other (git usually merges appends fine). The shown
entanglement is therefore an **upper bound**; true logical entanglement is lower. A **line-level**
pass (does the detour modify lines an earlier commit *added*, vs append new lines) would sharpen
each verdict — worth doing only for the PRs we actually choose to split.

## 9. Decisions needed before Phase 3

1. **Approve the normalization rules in §3** (or amend them), so I can generate the full 698-row
   mapping.
2. **Merge-narrative depth:** one-line summary per merge, or a short paragraph (what it enabled +
   notable commits)? The latter is the documentation you were after.
3. **Beads textual scope (§6):** scrub tooling/config only (recommended) vs full prose scrub.
4. **Doc resurrection target (§7):** restore into live `docs/` tree, or a `docs/archive/` marking
   them historical? (Pending the truly-dropped-vs-relocated manifest I'll generate.)
5. **Delivery: force-push in place, or replay into a fresh repo as real PRs?** This now also
   decides how §7 resurrection is done (don't-replay-deletion vs tip-commit).
6. **Pass 1 only, or also collapse PR #28?** (Recommend: Pass 1 now; decide #28 after.)

# Spec: ADRs drafted out of git, numbered at ship

- Status: proposed — awaiting approval before implementation
- Date: 2026-07-03
- Issue: TBD (file in `jaunder-org/jaunder` before implementing)
- Supersedes the authoring half of the always-0000 flow (ADR-0036 §#196); the
  collision-detection and `renumber` machinery it introduced are retained.

## Problem

The current flow **guarantees** ADR-number churn in git history. Two independent
faults compound:

1. **The number is assigned too early.** `jaunder-adr` step 5 runs
   `cargo xtask adr renumber` at _authoring_ time and the result is committed.
   But the correct number is only knowable at integration — `renumber` picks
   `max(ADRs reachable from origin/main) + 1` _as of that moment_. Any ADR that
   lands on `main` between that commit and the merge invalidates it. The
   assignment is provisional by construction, yet it is baked into a commit.

2. **The correction lands as a fixup, not an amend.** When
   `identifier-collisions` goes red after a rebase, the documented recovery is
   "run `renumber`", which `git mv`s the file; the author commits it _on top_.
   That is exactly the shape of
   `4edba19e docs(issue-210): renumber ADR 0045 -> 0046 after rebase onto main`
   — the number churn is visible in history because the fix is a new commit
   rather than being folded into the commit that introduced the ADR.

## Decision

Keep new ADRs **out of git** until ship. They are numbered at the last possible
moment — after the final rebase — so the ADR's _first appearance in git history
is already correctly numbered_, and the common case produces zero churn.

- **Author numberless drafts in `docs/adr/drafts/<slug>.md`.** The `drafts/`
  directory is gitignored, so a draft cannot be committed early. References to a
  draft during development are **path-form only** (`docs/adr/drafts/<slug>.md`);
  there is no bare draft token.
- **Number at ship.** A new `cargo xtask adr promote`, run in `jaunder-ship`
  after the final rebase, assigns each draft the next free number, moves it into
  `docs/adr/NNNN-<slug>.md`, rewrites its path-form references repo-wide, fixes
  its heading, syncs the README table, and stages everything. The ADR enters git
  correctly numbered, in one commit.
- **Residual race → amend, never fixup.** A collision is still possible between
  your ship commit and your merge (someone else's ADR merges first). The rule
  for that case is: re-rebase, re-run the assignment, and **amend the commit
  that introduced the ADR** — do not add a fixup commit. This is the one
  load-bearing discipline and it is documented in `jaunder-ship` and
  `CONTRIBUTING.md`.

### Why a `drafts/` subdirectory is nearly free

The three gates that police ADRs — `identifier-collisions`, `adr-format`,
`adr-readme-parity` — share one enumeration rule (`adr_readme::adr_files`,
`sequence_check::filenames`, `adr::adr_filenames`): **`is_file` → `.md` →
leading number**, via a **non-recursive** `read_dir` on `docs/adr/`. A
numberless draft under `docs/adr/drafts/` is excluded twice over — it is a
directory entry (not a file) _and_ it has no leading number. **No gate needs to
change.**

### The failure mode gets safer, not just rarer

If an author forgets to `promote`, the draft is untracked, so it simply never
reaches the PR — the failure is an ADR **absent** from the change (caught at
review, because the PR references a path that isn't there), not an ADR committed
with a **wrong** number that must be corrected in history later. We trade a
CI-enforced guard (which cannot see untracked files anyway) for a softer,
review-visible failure. `jaunder-ship` runs `promote` as a mandatory step so the
common path never hits this.

## Implementation plan

Ordered so the tree stays green at every commit. Rust/cargo, xtask conventions,
coverage policy apply throughout.

### 1. Holding pen + gitignore

- Create `docs/adr/drafts/` with a tracked `docs/adr/drafts/README.md` that
  explains the flow (why drafts live here, how `promote` graduates them).
- `.gitignore`:

  ```gitignore
  # ADR drafts live out of git until `cargo xtask adr promote` numbers them at
  # ship (see docs/adr/drafts/README.md). Keep the explainer tracked.
  docs/adr/drafts/*
  !docs/adr/drafts/README.md
  ```

- No gate change required (see above). Add a regression test asserting a
  numberless `docs/adr/drafts/foo.md` is invisible to `adr_files`,
  `sequence_check::filenames`, and `adr::adr_filenames`, locking that behavior
  against a future refactor of the enumeration rule.

### 2. Draft heading + template

- Draft canonical heading is `# ADR-DRAFT: <Title>`. `promote` swaps the single
  token `DRAFT` → `NNNN`. (Drafts are gate-invisible, so this heading is
  unconstrained until promotion.)
- Point the template at the draft flow: copy `docs/adr/template.md` to
  `docs/adr/drafts/<slug>.md`; change its first line to
  `# ADR-DRAFT: Title of the decision`. Everything else
  (Status/Date/Issue/Context/Decision/ Consequences) is unchanged.

### 3. `cargo xtask adr promote` (new subcommand, `xtask/src/adr.rs`)

Shares helpers with `renumber` (`ids::next_number`, `pad`, `rewrite_stem`,
`adr_readme::sync_readme_at`, `readme_has_markers`). New mechanics vs
`renumber`: the source is an **untracked** file, so it is create-new +
delete-old + `git add`, not `git mv`; and there is no bare-`ADR-NNNN` rewrite
(drafts have no bare token).

Behavior of `run_promote(repo)`:

1. Enumerate `docs/adr/drafts/*.md`, excluding `README.md`. Sort by slug for
   determinism. Empty → `"no ADR drafts to promote"` (a safe no-op, so
   `jaunder-ship` can always call it).
2. Load the current numbered ADRs (`adr_filenames`) as the base set.
3. **Two passes** (so a draft referencing another draft resolves):
   - Pass A — assign: for each draft in order, `n = ids::next_number(&all)`,
     compute `new_name = format!("{}-{}.md", pad(n), slug)`, push into `all`,
     record `(slug, n, new_name)`.
   - Pass B — apply each assignment:
     - Write `docs/adr/NNNN-<slug>.md` with the draft body, heading token
       `ADR-DRAFT` → `ADR-{pad(n)}`; remove `docs/adr/drafts/<slug>.md`.
     - Rewrite path-form references repo-wide: replace `drafts/<slug>` with
       `{pad(n)}-<slug>` (via `git grep -l --fixed-strings drafts/<slug>`,
       reusing the `rewrite_stem` substring replace — the slug makes it
       unambiguous, same assumption `renumber` already documents).
     - `git add` the new file and the removed draft path.
4. Sync the README table (`sync_readme_at` when `readme_has_markers`, else note
   the skip — mirror `renumber`).
5. Return a summary:
   `drafts/<slug>.md -> NNNN-<slug>.md; ...; README table synced (...)`. Unlike
   `renumber`, `promote` stages its output fully (the ADR is new, so there is
   nothing to review-before-add); state that in the summary.

Wire into `lib.rs` alongside `renumber`/`sync-readme`:
`cargo xtask adr promote`.

Tests (mirror the existing `renumber` tests' throwaway-git-repo harness):

- single draft → numbered, moved, staged, README row added, heading rewritten;
- two drafts → distinct consecutive numbers, deterministic by slug;
- path-form reference in a sibling file rewritten `drafts/<slug>` →
  `NNNN-<slug>`;
- draft-references-draft resolves to the promoted numbers;
- no drafts → clean no-op summary;
- promote after a branch already committed ADR NNNN picks NNNN+1.

### 4. `renumber` stays

`renumber` remains the recovery for the **residual race**: an ADR already
promoted-and-committed on the branch that collides after a _later_ rebase. Its
`git mv` + bare-ref-rewrite path is unchanged. Only the surrounding discipline
changes (amend, not fixup) — documented, not coded.

### 5. Docs + skills

- **`.claude/skills/jaunder-adr/SKILL.md`** — rewrite the authoring flow: copy
  template → `docs/adr/drafts/<slug>.md`; reference by draft path during
  development; at **ship** (post final rebase) `cargo xtask adr promote`; on a
  later collision `renumber` **and amend the introducing commit**. Keep it the
  single source of truth; `jaunder-start`/`jaunder-ship` point here.
- **`.claude/skills/jaunder-start/…`** — when a task needs an ADR, create the
  draft in `drafts/` (not a numbered file).
- **`.claude/skills/jaunder-ship/…`** — add a mandatory step: after the final
  rebase, if `docs/adr/drafts/*.md` exist, run `cargo xtask adr promote`, stage,
  include in the ship commit, and verify the gate is green; document the
  amend-on-late-collision rule.
- **`CONTRIBUTING.md` "Adding an ADR"** — replace the always-0000 description
  with the draft → promote → (amend on late collision) flow. Keep the
  gate/`sync-readme`/collision paragraphs; note that drafts are gate-invisible
  by construction.

### 6. Record the decision as an ADR (dogfood)

Author the ADR below as `docs/adr/drafts/adr-out-of-git-draft-workflow.md` and
`promote` it as part of shipping this change — the first exercise of the new
flow. Fill its Issue field once the tracker issue exists; supersede/annotate
ADR-0036's authoring half per the ADR-status rules.

## Residual risks / open points

- **Discoverability of drafts.** Untracked files don't show in a fresh clone;
  the tracked `drafts/README.md` and the skill/CONTRIBUTING edits are the
  mitigations. Acceptable.
- **Slug collision between a draft and an existing ADR slug** would make the
  path-form rewrite over-match (same caveat `rewrite_stem` already documents).
  Vanishingly unlikely across independently-authored decisions; tighten to a
  boundary match only if it ever bites.
- **`promote` is not idempotent by identity** (re-running after a re-rebase
  re-assigns from the new base). That is the intended behavior for the
  amend-on-late-collision path; the skill spells out "re-run, then amend."

---

## Proposed ADR (drop-in body for `docs/adr/drafts/adr-out-of-git-draft-workflow.md`)

```markdown
# ADR-DRAFT: ADRs drafted out of git, numbered at ship

- Status: proposed
- Date: 2026-07-03
- Issue: [#TBD](https://github.com/jaunder-org/jaunder/issues/TBD)

## Context

ADR numbers are a shared monotonic sequence. Two branches authoring an ADR
concurrently both want "the next number", and the only moment the correct number
is knowable is at integration. The prior flow (ADR-0036, #196) handled this by
authoring every ADR as the `0000` sentinel and running
`cargo xtask adr renumber` to reconcile — but `renumber` was run at _authoring_
time and its result committed, so the number was assigned before it was final.
When a rebase later revealed a collision, the recovery (`renumber` again) landed
as a _new_ commit on top of the one that introduced the ADR. The result was
guaranteed number churn in git history (e.g. "renumber ADR 0045 -> 0046 after
rebase onto main").

## Decision

New ADRs live **out of git** until ship, and are numbered at the last possible
moment:

- Authored numberless as `docs/adr/drafts/<slug>.md`; the `drafts/` directory is
  gitignored, so a number cannot be committed early. Drafts are referenced by
  path (`docs/adr/drafts/<slug>.md`) during development.
- `cargo xtask adr promote`, run in `jaunder-ship` after the final rebase,
  assigns the next free number, moves the draft to `docs/adr/NNNN-<slug>.md`,
  rewrites its path-form references, syncs the README table, and stages
  everything. The ADR's first appearance in history is already correctly
  numbered.
- If a collision still surfaces between the ship commit and the merge,
  re-rebase, re-run the assignment, and **amend the commit that introduced the
  ADR** — never add a fixup commit. `renumber` remains the tool for this
  already-committed-ADR case.

The three ADR gates (`identifier-collisions`, `adr-format`, `adr-readme-parity`)
require no change: their shared enumeration rule (`is_file` → `.md` → leading
number, non-recursive over `docs/adr/`) already excludes a numberless draft in a
subdirectory.

## Consequences

- No ADR number churn in normal history: the number is written to git exactly
  once, at ship.
- Forgetting to `promote` yields an ADR _absent_ from the PR (caught at review),
  not a _wrong_ number in history — a safer failure mode. `jaunder-ship` runs
  `promote` unconditionally to keep the common path clean.
- The "did you take someone's number?" guard for the residual race moves from a
  CI gate (which can't see untracked drafts) to a documented amend discipline in
  `jaunder-ship`.
- Drafts are invisible in a fresh clone; a tracked `docs/adr/drafts/README.md`
  and the skill/CONTRIBUTING docs carry the flow.
```

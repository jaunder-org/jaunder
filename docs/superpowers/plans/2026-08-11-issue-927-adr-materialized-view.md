# Issue #927 — ARCHITECTURE.md as the materialized view of the ADR log

Spec: [issue #927](https://github.com/jaunder-org/jaunder/issues/927). This plan
is "how"; the issue is "what and why". Do not re-narrate it.

## Review header

**Goal.** Make `docs/ARCHITECTURE.md` the authoritative, ADR-cited statement of
the current architecture, then lock that state with a stateless
`adr-view-parity` gate and a `jaunder-adr-projection` skill so it cannot rot
again.

**Scope — in:** the view rebuild; the consolidated "Un-ADR'd reality" section;
the ADR-0052 drift annotation; tense correction on 13 ADRs; the gate; the skill;
promotion of the parked draft ADR; the `CONTRIBUTING.md` trim;
`docs/adr/template.md` and the ADR-0023 seam fix.

**Scope — out:** relocating addendum content into the view (tense only);
superseding ADR-0000; a diff-based "changed alongside" check; `CONTEXT.md`
content backfill; `ROADMAP.md`; `hub-architecture.md`; filling the ADR gaps
themselves.

**Tasks.**

This is the authoritative progress list — tick here as you go. Tasks 3–14 are
also listed individually further down.

- [x] **1** — Restore the parked starting texts onto the current base; repair
      dead links. _(no repair needed — all 161 links resolved)_
- [x] **2** — Build the ADR-to-section assignment worksheet for all accepted
      ADRs. _(108 assigned, validated as an exact partition; grew to 14
      sections)_
- [ ] **3–14b** — Re-verify and rebuild each of the 14 view sections (one task,
      one commit each).
- [ ] **15** — Parity sweep: place every straggler ADR the section pass missed.
- [ ] **16** — File the ADR-gap follow-up issues (their numbers are needed by
      task 17).
- [ ] **17** — Add the "Un-ADR'd reality" section.
- [ ] **18** — Annotate ADR-0052's drifted check inventory.
- [ ] **19** — Tense-correct the 13 ADRs carrying present-tense addenda.
- [ ] **20** — Amend the draft ADR (status, issue link, un-ADR'd wording).
- [ ] **21** — Re-derive the `CONTRIBUTING.md` trim against today's file.
- [ ] **22** — Add the `jaunder-adr-projection` skill and wire its two pointers.
- [ ] **23** — Add the `adr-view-parity` step — **final commit, lands green**.

**Key risks and decisions.**

- **The corpus moves under this branch, and the main checkout lags the
  worktree.** Task 2 derived **114 files, 108 accepted, highest 0113** from the
  _worktree_. An earlier count taken from the main checkout was already stale by
  a commit — always read the worktree. Re-derive again at task 15; never trust a
  number written in this plan or the issue body. (The issue says "highest 0112,
  114 files"; 0113 has since landed and 0074 became `superseded`, so the totals
  coincide by accident.)
- **A sloppy fan-out agent produces confident wrong prose that reads exactly
  like correct prose.** This already happened once — a wrong "seam moved" claim
  landed in ADR-0023. Every section gets a second pass by a different agent.
  Task 15's probe catches missing citations but _cannot_ catch a false claim;
  the second pass is the only thing that does.
- **The view grew from 12 sections to 14** — resolved at task 2. A cross-cutting
  type-safety cluster got its own "Domain types and invariants" section (10
  ADRs), and "Testing and verification gates" split into the suite (17) and the
  ladder (10), because at 27 it was more than twice any other section. Task 14b
  writes new prose; every other section task rewrites parked text.
- **The gate must land last and green.** Any earlier ordering red-lights every
  commit on the branch.
- **The skill is not a PR deliverable.** `.claude/` is untracked in jaunder, so
  task 22 lands in the main checkout only — unreviewed, and live immediately for
  every other worktree. The PR therefore delivers the view and the gate; the
  skill ships beside it, out of band.
- **The draft ADR is gitignored and was the sole copy.** It is now backed up at
  `/home/mdorman/src/jaunder/.parked-backup-927/`. Do not delete that directory
  until the PR merges.
- **SHIP BLOCKER — `promote` MUST run before the branch is pushed.** The view
  links to `adr/drafts/architecture-view-materialized-from-adrs.md` in several
  places. `doc-links` resolves targets with `.exists()` on disk
  (`xtask/src/doc_links.rs:209`), and the drafts pen is gitignored — so those
  links pass locally, where the file is present, and would **fail in a CI clone,
  where it is not**. The designed flow already covers this:
  `cargo xtask adr promote` rewrites every `drafts/<slug>` path-form reference
  repo-wide to `NNNN-<slug>` and stages the result, so CI never sees a draft
  link. But it means the ordering is not optional. Sequence at ship: final
  rebase → `promote` → gate → push → PR. If the branch is ever pushed before
  `promote`, CI fails on `doc-links` and the cause will not be obvious from the
  error.

**For agentic workers.** Drive with `jaunder-iterate`; delegate individual
section tasks (3–14) via `jaunder-dispatch`. Tick checkboxes in real time.

## Global constraints

- **Docs-only for tasks 1–21.** No Rust changes until task 23.
- **Skills live outside git, and outside this worktree.** `.claude/` is
  untracked in jaunder — zero files in the index, not gitignored, just never
  added — and the worktree has no `.claude/` directory at all. Every skill edit
  therefore targets the **main checkout by absolute path**:
  `/home/mdorman/src/jaunder/.claude/skills/`. Task 22 produces **no commit**
  and does not appear in the PR. Because the skills are shared across every
  worktree, editing `jaunder-adr` or `jaunder-commit` takes effect immediately
  for any other session in flight.
- **Per-commit gate:** run
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/adr-materialized-view -- cargo xtask check`
  before each `git commit`, foreground, `timeout: 600000`. The first run in this
  worktree is cold — xtask compiles, then the gate runs cold. See
  `jaunder-commit`.
- **`prettier` reformats markdown.** `cargo xtask check` auto-fixes and
  fails-and-restages; `git add` and re-commit. Use the devShell's `prettier`,
  never `npx`.
- **`doc-links` is a hard gate.** Every relative markdown link in a touched file
  must resolve. This is the most likely per-commit failure for tasks 1–21.
- **No commit trailers.** No `Co-Authored-By`.
- **Reference the issue inline:** `(#927)` in the subject or body.
- **Do not touch `docs/README.md`.** Its ADR table is a generated projection
  owned by `cargo xtask adr promote` / `adr sync-readme`.
- **Never hand-edit an ADR number.** The draft stays numberless until `promote`
  at ship.

## The section brief (tasks 3–14 all use this)

Each section task runs two agents in sequence. Both get the section's assigned
ADR list from task 2's worksheet.

**Pass A — draft.** For the section's subject area:

1. Read the parked text for this section as the starting point. Treat every
   sentence as a claim to be checked, not as prose to be preserved.
2. Verify each claim against current code. Cite the file and line you checked. A
   claim you cannot verify is deleted or rewritten, never softened into
   vagueness.
3. Ensure every accepted ADR assigned to this section is cited at least once, in
   the form `[ADR-NNNN](adr/NNNN-slug.md)`.
4. Keep **current reality** separate from **committed direction**. Aspirational
   ADRs (ingestion, federation) belong under an explicit direction heading so an
   unbuilt subsystem never reads as shipped.
5. Classify every `un-ADR'd` flag inherited from the parked text, and any new
   one you find, into exactly one of: **noise** (delete it, no trace), **drift**
   (an ADR reality has outrun — report it, do not fix it here), **gap**
   (undecided and architecturally significant — report it for task 16/17),
   **decided elsewhere** (cite the issue inline in the prose).
6. Collect domain vocabulary this section's ADRs introduced that `CONTEXT.md`
   lacks. Report the list; do not edit `CONTEXT.md`.

**The superseded-ADR rule — applies to both passes.**

**Never cite a `superseded` ADR as establishing current practice.** Cite its
successor. A superseded ADR may appear only in an explicitly past-tense or
historical construction ("ADR-0055's module-level split, superseded by ADR-0070,
survives in the unmigrated verticals"). The parity gate cannot catch this — it
only checks that accepted ADRs are _present_ — so it is on you.

Found at task 3, inherited from the parked text, in six places. Note the 0043
case especially: it is not merely a stale citation, it describes a
`[patch.crates-io]` / flake-input / crane-override apparatus that has since been
**deleted outright**, so the Protocols (line ~325) and Development tooling (line
~1026) sections currently assert machinery that no longer exists.

| Superseded | Successor                                                                                                             |
| ---------- | --------------------------------------------------------------------------------------------------------------------- |
| 0013       | 0070                                                                                                                  |
| 0030       | 0050 (the stateless coverage gate)                                                                                    |
| 0043       | 0089 — and the machinery is GONE                                                                                      |
| 0055       | 0056 (NOT 0070 — and 0070 is a deliberate partial _return_ to 0055's module-level gating; 0055's status is unchanged) |
| 0056       | 0070                                                                                                                  |
| 0074       | 0075                                                                                                                  |

**Pass B — verify.** A _different_ agent, which does not see Pass A's reasoning:

1. Re-check every factual claim in the section against code, independently.
2. Specifically re-verify any claim that something _moved_, was _renamed_, or
   was _retired_ — this is the failure class that already reached production in
   ADR-0023.
3. Confirm every assigned ADR is cited and no dead ADR number is referenced.
4. Report disagreements; do not silently rewrite.

Pass B's disagreements are resolved before the commit, by reading the code
yourself.

**Deletion is not the safe default — learned at task 12.** A section pass
deleted three claims it could not verify. Two were true and sourced: ADR-0031:83
states "a test per pure function" verbatim, and `runtime.json` really is a
startup mutex (`check_startup_mutex` makes `serve` refuse against a live writer,
#141). A third was true but belonged elsewhere. "I could not verify it" is a
reason to _investigate_ or to hand the claim to the owning section — deleting it
silently loses content just as surely as asserting a falsehood adds it. Say what
you removed and why, always.

**Operational note, learned at task 3.** Both agents finished their work and
went idle **without sending their report**, costing a round-trip each. End every
section brief with an explicit instruction: _"When you are done, you MUST call
SendMessage to `team-lead` with your report. Going idle without sending it loses
your work."_ State the report's required shape in the brief, and tell Pass B to
lead with what is wrong rather than with what it confirmed.

**Commit:**
`docs(architecture): rebuild the <section> section of the view (#927)`

## Tasks

### Task 1 — restore the parked starting texts onto the current base

**Files**

- `docs/ARCHITECTURE.md` — replace with the parked 1068-line text.
- `docs/adr/template.md` — apply the parked past-tense-addendum convention.
- `docs/adr/0023-atompub-jaunder-wire-extensions.md` — apply the parked seam
  correction: `format_wire` is `server/src/atompub/mapping.rs` and never moved;
  only namespace and slug definitions live in `common/src/atompub`.

Recover all three from the snapshot tag:

```bash
git checkout parked-927-snapshot -- docs/ARCHITECTURE.md docs/adr/template.md \
  docs/adr/0023-atompub-jaunder-wire-extensions.md
```

**Do not** restore `CONTRIBUTING.md` from the tag — it is re-derived in task 21
against today's 1020-line file.

**Then repair what four months broke.** The parked text links into a tree that
has moved. Run the gate and fix every dead link it names:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/adr-materialized-view -- cargo xtask check
```

Verify against `.xtask/last-result.json` that `doc-links`, `adr-format`, and
`adr-readme-parity` are `ok`.

**Commit.**
`docs(architecture): restore the parked materialized view onto current main (#927)`

The commit message must state plainly that this text is the _starting point_ and
its claims are verified section by section in the commits that follow — it is
not yet trustworthy, and the history should say so.

### Task 2 — ADR-to-section assignment worksheet

**Files:** this plan, appendix.

Enumerate every ADR with status `accepted` and assign each to exactly one of the
view's sections. Re-derive the corpus rather than trusting any count written
here.

Decide the type-safety cluster (0072, 0073, 0074, 0075, 0091, 0101, 0108): a
13th section, or distributed. Recommendation is a 13th section, "Domain types
and invariants"; if you conclude otherwise, say why in the worksheet.

Every accepted ADR must appear exactly once. That property is what makes task
23's gate green by construction rather than by luck.

Also record, for each section, its inherited `un-ADR'd` flags from the parked
text.

**Commit.** `docs(927): assign every accepted ADR to a view section (#927)`

### Tasks 3–14 — rebuild the twelve (or thirteen) sections

One task and one commit per section, each following **The section brief** above.

- [x] Task 3 — Workspace. Added the `client` row and the separate `tools/`
      workspace; replaced the "reserved future" framing with the actual
      split-by-compile-target rule. Pass B found no defects.
- [x] Task 4 — Storage. Pass A deleted a nonexistent symbol claim; Pass B then
      found three more errors in Pass A's own draft. ADR-0006 confirmed unbuilt
      by both passes independently.
- [ ] Task 5 — Content model.
- [ ] Task 6 — Protocols (AtomPub, feeds, WebSub).
- [ ] Task 7 — Authentication.
- [ ] Task 8 — Web frontend. Largest post-park ADR load.
- [ ] Task 9 — Observability.
- [ ] Task 10 — Deployment.
- [ ] Task 11 — Emacs client.
- [ ] Task 12 — Testing: harness, suites, e2e (17 ADRs).
- [ ] Task 12b — Verification gates: the ladder (10 ADRs).
- [ ] Task 13 — Development tooling.
- [ ] Task 14 — Documentation and decision process.
- [ ] Task 14b — Domain types and invariants. Task 2 created this section; it is
      new prose, not a rewrite, so there is no parked text to start from.

### Task 15 — parity sweep

**Files:** `docs/ARCHITECTURE.md`.

Re-derive the accepted-ADR set and the set cited by the view, and list the
difference. Place every straggler in the section where it belongs; a straggler
means task 2's worksheet was incomplete, so fix the worksheet too.

This probe is the same computation task 23 encodes as a gate. Reaching zero here
is what lets task 23 land green.

**Also sweep the opposite direction:** list every citation of a `superseded` ADR
and confirm each sits in an explicitly past-tense or historical construction,
never one asserting current practice. See section-brief rule 4b. The gate is
blind to this.

**This probe cannot detect a false claim** — only a missing citation. It is not
a substitute for the section briefs' Pass B.

**Commit.**
`docs(architecture): cite every remaining accepted ADR in the view (#927)`

### Task 16 — file the ADR-gap follow-up issues

Use **`jaunder-issues`**. One issue per gap unless several are plainly one
decision. Known candidates, to be re-derived from what tasks 3–14 actually
reported:

- Content-addressed media store (sha256 pathing) — assumed by ADR-0024, never
  decided.
- Publisher-side WebSub — ADR-0010 names WebSub only as an ingestion channel.
- Hashed-at-rest token storage, and the `cheap-kdf` feature with its dual
  fail-closed guard.
- The embedded SPA shell versus on-disk wasm bundle split (#239), recorded only
  in code comments and `flake.nix`.
- Cookie, then Bearer, then Basic credential precedence.
- The NixOS module and package outputs.

Also file the two non-gap follow-ups: the `docs/archive/` practice has no ADR
(ADR-0000 says _delete_, practice archives), and the `CONTEXT.md` backfill —
seeded with the vocabulary worksheet tasks 3–14 collected.

Record the issue numbers; task 17 needs them. No commit.

### Task 17 — the "Un-ADR'd reality" section

**Files:** `docs/ARCHITECTURE.md`.

Add a final section listing every surviving gap with its issue number, and a
one-line preamble saying what the list is: claims the view makes that no ADR
establishes, each either a decision worth recording or detail not worth an ADR,
with the issue saying which.

No inline `un-ADR'd` comments survive this task. Noise was deleted during the
section passes; drift went to task 18; decided-elsewhere items were cited
inline.

**Commit.**
`docs(architecture): list un-ADR'd reality with tracking issues (#927)`

### Task 18 — annotate ADR-0052's drifted inventory

**Files:**
`docs/adr/0052-devtool-single-implementation-of-non-compiling-checks.md`
(confirm the exact filename).

ADR-0052's decision holds; only its inventory drifted, from 7 non-compiling
checks to 8 — `byte-compile` was added and the former tsc-deps step folded into
`devtool check tsc`. Add a short **past-tense** annotation recording that,
pointing at the view's Development tooling section for the current inventory. Do
not edit its Decision text.

Re-verify the 7-to-8 claim against `xtask/` and `tools/` before writing it; the
count is exactly the kind of fact that has already drifted once.

**Commit.**
`docs(adr): annotate ADR-0052's check inventory as historical (#927)`

### Task 19 — tense-correct the addendum ADRs

**Files:** the 13 ADRs carrying present-tense addendum, amendment, or supplement
sections. Re-derive the list; it was 13 at planning time, worst cases ADR-0011
(5 addenda), ADR-0016 (4), ADR-0030 (3).

**Wording only.** "The facade therefore moves to X" becomes "as of 2026-07-09
the facade moved to X". Every clause of reasoning stays exactly where it is.
`ADR-0033`'s `## History` section is the exemplar of the target voice.

**Explicitly forbidden:** moving addendum content into the view. Those addenda
hold genuine rationale, and rationale is what an ADR is for.

Where an addendum's subject is now described in the view, append a pointer to
the relevant section — an addition, not a replacement.

**Commit.** `docs(adr): rewrite present-tense addenda in past tense (#927)`

### Task 20 — amend the draft ADR

**Files:** `docs/adr/drafts/architecture-view-materialized-from-adrs.md`
(gitignored; backup at `/home/mdorman/src/jaunder/.parked-backup-927/`).

1. `- Status: accepted` becomes `- Status: proposed`. The pen _is_ the proposed
   state and `promote` records acceptance when it assigns the number (ADR-0088).
2. Add an `- Issue: [#927](https://github.com/jaunder-org/jaunder/issues/927)`
   line, matching the sibling draft's shape.
3. Update the Date.
4. Soften the "flagged, then either ADR'd or corrected — not silently kept"
   clause to match what shipped: un-ADR'd reality is listed explicitly in the
   view's final section and tracked as issues. As written, the very first
   shipment of the view breaks the rule the ADR states.
5. Keep the narrow "amends ADR-0000" framing. Do **not** supersede ADR-0000.
6. Check its links against `docs/adr/drafts/README.md` steps 4 and 5 — sibling
   ADRs bare, another draft as `../drafts/slug.md`. Promotion strips one
   relative level, so the form that survives is not the form that resolves in
   the pen.

No commit — the draft is gitignored. It enters git at `cargo xtask adr promote`,
run by `jaunder-ship` after the final rebase.

### Task 21 — re-derive the CONTRIBUTING.md trim

**Files:** `CONTRIBUTING.md` (1020 lines today).

Re-derive against the current file. Do **not** merge the parked diff — its base
is four months and 23 commits stale. The parked version at `parked-927-snapshot`
is a reference for _intent_ only.

Apply:

- Repository layout section becomes a pointer to the view's Workspace section,
  plus the non-crate directories.
- The `docs/ARCHITECTURE.md` line in Project guides describes it as the
  materialized view of the ADR log.
- Architectural content (the DI block, storage conventions that restate ADR-0016
  and ADR-0019) becomes pointers into the view. Process content stays.
- `beads` / `bd` references (around line 102) become GitHub issues.
- The commit-trailer mandate (around lines 844-847) is **deleted**. Both
  trailers it requires are dead: beads is gone, and `Co-Authored-By` is
  forbidden. Replace with the convention actually in force — reference the issue
  inline, for example `(#875)`.
- Fix the stale paths: backends live in `storage/`, and web server functions
  retrieve the per-trait handle rather than the whole `AppState` bundle.

**Commit.**
`docs(contributing): trim to process and point at the architecture view (#927)`

### Task 22 — the `jaunder-adr-projection` skill

**Files — all in the MAIN checkout, by absolute path, never in this worktree.**

- `/home/mdorman/src/jaunder/.claude/skills/jaunder-adr-projection/SKILL.md` —
  new.
- `/home/mdorman/src/jaunder/.claude/skills/jaunder-adr/SKILL.md` — invoke it
  after the draft is written.
- `/home/mdorman/src/jaunder/.claude/skills/jaunder-commit/SKILL.md` — a short
  pointer for the supersession case.

`.claude/` is untracked in jaunder, so **this task has no commit** and its work
is not in the PR. Two consequences to accept deliberately: the skill gets no
code review, and the edits to `jaunder-adr` / `jaunder-commit` are live
immediately for every other worktree and session.

The skill carries **both** disciplines the ADR commits to:

- **Online projection**, per ADR, fired at draft authoring. Explain the
  mechanism that makes this cheap: edit the view while the draft is still
  numberless, citing it as `docs/adr/drafts/slug.md`; `run_promote` Pass C
  (`xtask/src/adr.rs:358`) rewrites that path repo-wide to the assigned number
  and stages it. Parity is then green for free. Editing the view at commit or
  ship time instead means doing it at the worst moment, because a new ADR does
  not exist as a numbered file until `promote` runs.
- **Periodic replay**, the occasional full audit that re-derives the view from
  the log plus the code. This half having no written procedure is why this work
  rotted for four months. Reuse the section brief above as the procedure.

Document scope, and be explicit that "mandatory" means the skill must ask and
answer, not that every ADR edits the file:

| Document                   | Rule                               |
| -------------------------- | ---------------------------------- |
| `docs/ARCHITECTURE.md`     | Mandatory — and gated              |
| `CONTEXT.md`               | Mandatory consideration            |
| `CONTRIBUTING.md`          | Conditional — process ADRs         |
| `docs/DESIGN.md`           | Conditional — user-facing behavior |
| `docs/README.md`           | Forbidden — generated by `promote` |
| `docs/ROADMAP.md`          | Out of scope                       |
| `docs/hub-architecture.md` | Out of scope                       |

Also state what the gate does **not** catch, so the skill covers it: a
superseded or materially rewritten ADR whose number is already cited keeps
parity green while the view still describes the dead decision. That case is the
reason for the `jaunder-commit` pointer.

**No commit** — see the Files note above.

### Task 23 — the `adr-view-parity` gate (final commit)

**Files**

- `xtask/src/adr_readme.rs` — add
  `view_parity_problems(repo: &Path) -> Result<Vec<String>>`, beside the
  existing `parity_problems` / `parity_report`. Tests are in-file
  `#[cfg(test)]`, matching this module's existing 22.
- `xtask/src/steps/adr_check.rs` — add `view_parity_step()`, pushed from the
  existing `run`.
- `xtask/src/lib.rs` — no new registration needed; `steps::adr_check::run` is
  already called at both sites (lines 458 and 505).
- `CONTRIBUTING.md` — document the new step beside `adr-format` and
  `adr-readme-parity`.

**Semantics.** Every ADR whose status is `accepted` must be cited at least once
in `docs/ARCHITECTURE.md`. `superseded`, `deprecated`, and `rejected` are
excluded. No allowlist, no exemption file, no baseline — a pure function of the
tree, consistent with the coverage gate's stance in ADR-0050. `parse_adr_dir`
already yields `AdrEntry { num, status, .. }`, so the ADR side is free.

A citation is a link to `adr/NNNN-slug.md` or the bare `ADR-NNNN` token. Accept
both; the view uses links, and prose sometimes uses the bare token.

**Tests** (in-file, `xtask/src/adr_readme.rs`), written before the
implementation:

- an accepted ADR cited by link — no problem reported;
- an accepted ADR cited by bare `ADR-NNNN` token — no problem reported;
- an accepted ADR cited nowhere — reported, and the message names the number;
- a `superseded` ADR cited nowhere — **not** reported;
- a view citing a number with no ADR file — not this step's business, no
  problem;
- an absent `docs/ARCHITECTURE.md` — an error, not a silent pass.

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/adr-materialized-view -- cargo nextest run -p xtask view_parity
```

Expect FAIL before the implementation, PASS after.

**Then run the real gate and confirm it is green on this branch** — that is the
issue's exit criterion:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/adr-materialized-view -- cargo xtask check
```

`adr-view-parity` must be `ok` in `.xtask/last-result.json`. If it names uncited
ADRs, the section work is incomplete: go back and place them. **Do not** add an
exemption to make the gate pass — the absence of an escape hatch is the whole
design.

**Commit.**
`build(xtask): gate that every accepted ADR is cited in the view (#927)`

## Appendix — ADR-to-section worksheet

Derived from the **worktree** tree (not the main checkout, which lags) at task
2: **114 ADR files, 108 `accepted`, 6 `superseded` (0013 0030 0043 0055 0056
0074), highest 0113.** 48 accepted ADRs are uncited by the restored view — the
gap task 23 closes.

Validated mechanically: every accepted ADR appears **exactly once**, no
duplicates, no strays. That property is what makes task 23 green by
construction. Re-run the check if you move anything.

Two structural decisions taken here:

- **A 14th section, "Domain types and invariants"**, for the ten cross-cutting
  newtype/invariant ADRs that have no subsystem home. Without it, 0072
  (UtcInstant), 0073 (`url`), 0091 (`#[text_enum]`), 0101 (infallible kind) and
  0108 (absence named at its source) each get filed under whichever subsystem
  happens to use them, which is how they became invisible in the first place.
- **"Testing and verification gates" splits in two.** At 27 ADRs it was more
  than twice any other section. The suite and the ladder are different subjects
  with different readers: one is "how we test", the other is "what blocks a
  commit".

| Section                            | Task | n   | Accepted ADRs                                                                        |
| ---------------------------------- | ---- | --- | ------------------------------------------------------------------------------------ |
| Workspace                          | 3    | 3   | 0058 0062 0069                                                                       |
| Storage                            | 4    | 7   | 0001 0006 0016 0019 0021 0064 0092                                                   |
| Content model                      | 5    | 13  | 0004 0005 0009 0020 0024 0025 0027 0079 0080 0084 0090 0097 0105                     |
| Protocols (AtomPub, feeds, WebSub) | 6    | 5   | 0010 0015 0023 0089 0112                                                             |
| Authentication                     | 7    | 5   | 0007 0014 0018 0022 0107                                                             |
| Web frontend                       | 8    | 16  | 0002 0040 0041 0044 0059 0060 0061 0065 0070 0076 0078 0082 0083 0093 0106 0113      |
| Observability                      | 9    | 4   | 0011 0049 0096 0100                                                                  |
| Deployment                         | 10   | 3   | 0003 0008 0102                                                                       |
| Emacs client                       | 11   | 6   | 0031 0035 0038 0042 0045 0047                                                        |
| Testing (harness, suites, e2e)     | 12   | 17  | 0012 0026 0032 0033 0034 0037 0039 0046 0051 0053 0054 0057 0067 0098 0099 0103 0111 |
| Verification gates (the ladder)    | 12b  | 10  | 0029 0050 0066 0081 0085 0086 0094 0095 0109 0110                                    |
| Development tooling                | 13   | 5   | 0028 0052 0077 0087 0104                                                             |
| Documentation and decision process | 14   | 4   | 0000 0036 0048 0088                                                                  |
| Domain types and invariants        | 14b  | 10  | 0017 0063 0068 0071 0072 0073 0075 0091 0101 0108                                    |

**Judgement calls worth a second opinion during the section passes.** Each is
defensible either way; if the section agent disagrees on evidence, move it and
say so.

- **0016** (DI / `AppState` / composition root) → Storage, following the pointer
  `CONTRIBUTING.md` already makes. It is arguably a Workspace-wide invariant.
- **0086** (component thinness enforced) → Verification gates, not Web frontend:
  the ADR is about the enforcement, not the components.
- **0106** (wasm size budget) → Web frontend, as a bundle-delivery concern,
  though it is enforced as a gate and could sit in Deployment.
- **0082** (server-fn wire URLs) → Web frontend rather than Protocols: it is the
  internal RPC surface, not a published protocol.
- **0071** (sqlx newtype bridge) → Domain types, not Storage: the subject is the
  newtype, and storage is where it happens to cross.
- **0112** (role-tagged site URLs) → Protocols, as part of the published URL
  surface.

### Running findings — gaps, drift, and disposals

Accumulated by the section passes. Tasks 16 and 17 consume the GAP rows; task 18
consumes the DRIFT rows. Append as each section reports.

**GAPs — real, architecturally significant, no ADR.**

| Gap                                                                                                                                                                                                                                                                                                                                          | Section   |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| **Local soft delete.** `soft_delete_post` stamps `deleted_at`; ~30 read sites filter it. `rg` over `docs/adr/` finds nothing. It is the local counterpart of ADR-0009's inbound never-purge rule, and no ADR states the policy or whether a hard delete ever happens. _(Pass B checking the issue tracker before this is final.)_            | Content   |
| **Local revision snapshots.** Every update writes an immutable `post_revisions` row, but no ADR decides it. ADR-0009 looks like the governing decision and is not: it speaks only of consumed content ("for followed sources", "when an update is received"). Write-only today — no read query, no surface.                                  | Content   |
| **Publisher-side WebSub.** Built (`server/src/websub/{mod,http,noop,file_capture}.rs`) and exercised by e2e, yet no ADR decides it — ADR-0010 names WebSub only as a future _ingestion_ channel.                                                                                                                                             | Protocols |
| **`cheap-kdf`.** Weakens Argon2 to `MIN_M_COST`/t=1 for test speed, held off production by two guards (compile-time `compile_error!` on `not(debug_assertions)`, startup `exit(1)`). **Zero ADRs and zero GitHub issues mention it** — the strongest gap found.                                                                              | Auth      |
| **Hashed-at-rest token storage.** A decision was _declined_, not made: #554 proposed encoding hash-before-store in the type system and was closed `not_planned`, leaving it "a convention, enforced only by `RawToken` carrying `#[str_newtype(no_sqlx)]`". Security-load-bearing and ADR-silent.                                            | Auth      |
| **Credential precedence order.** Cookie beating an explicit `Authorization` header is a confused-deputy decision nothing makes. ADR-0007 is 38 lines, names cookies and Bearer, ranks nothing, and never mentions Basic at all. (The _homing_ of `resolve_credential` and the empty-cookie fall-through are decided-elsewhere — #334, #344.) | Auth      |
| **Lowercase-canonical username.** #67 and #344 act on the rule as settled without establishing it; ADR-0063 mandates the validating-newtype shape, not this normalization.                                                                                                                                                                   | Auth      |
| **The cargo-workspace exclusions.** Root `exclude = ["xtask"]` and the separate `tools/` workspace are stated only in flake comments and ADR asides. Nearest issue is #276, which _assumes_ the tools workspace rather than deciding it.                                                                                                     | Tooling   |
| **The `lettre` fork patch.** The one surviving `[patch.crates-io]` entry (`Cargo.toml:134`), un-ADR'd — the same decision class ADR-0043 once recorded for the atom forks.                                                                                                                                                                   | Tooling   |
| **`host_tests`** (`xtask-tests`, `tools-test`). No ADR names either step. Minor — it is "run the unit tests of the two workspaces Nix excludes".                                                                                                                                                                                             | Gates     |
| **Feed item-selection and cache validation.** `HybridWindow` (`common/src/feed/window.rs`, defaults 20 items / 30 days) and `feed_etag` conditional GET are real architecture with no ADR.                                                                                                                                                   | Protocols |

**DRIFT — an ADR reality has outrun.**

| Drift                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Owner   |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------- |
| **ADR-0052 (`:41`, `:52`)** says 7 non-compiling checks. `tools/devtool/src/check.rs:17` `pub const ALL` holds **8** — fmt, leptosfmt, prettier, tsc, elisp-fmt, ert, byte-compile, tools-fmt. `byte-compile` was added; the former tsc-deps step folded into `devtool check tsc` (`check.rs:110-114`). Host side matches one-for-one.                                                                                                                                                                 | Task 18 |
| ~~**ADR-0028** frames `devtool` as sandbox-only while `run`/`check` are host-invoked.~~ **WITHDRAWN.** ADR-0028's own Supplement (#158, `:96-117`) extends `devtool run` to the host as "the gate-execution surface for humans and agents" and exposes `devtoolBin` in the devShells; `devtool check`'s host invocation is ADR-0052's Decision (`:45-47`). Both are chartered. The Context paragraph is superseded inside its own document. Second withdrawn drift of the run, after ADR-0089.         | —       |
| **ADR-0124's second bullet is false for this tree.** It claims `use rstest_reuse::*` alone is insufficient at a crate root and that a bare `use rstest_reuse;` must also be present. No such import exists anywhere in `server/` or `storage/` — `server/tests/main.rs` imports no rstest_reuse at all — and the suite compiles. Probably a stale comment hoisted verbatim by the #930 audit, or true before the single-binary test crate. **Landed 2026-08-12, hours old.** Not asserted in the view. | Task 18 |
| **ADR-0036's addendum (`:71-72`)** still lists the **five**-token status set including `proposed`. ADR-0088 narrowed a numbered ADR to four, and `adr-format` rejects `proposed` outright. The view cites 0088 and is correct; 0036 is a stale-looking source a reader may land on first.                                                                                                                                                                                                              | Task 18 |
| **ADR-0073** names `AbsoluteUrl` as the type holding the `url::Url` chokepoint. That type is deleted — ADR-0112 replaced it with `TaggedUrl<T>` (`common/src/tagged_url.rs:73`, 15 roles). ADR-0073's actual decision is intact; only the type name is stale, and 0073 carries no pointer to 0112.                                                                                                                                                                                                     | Task 18 |
| **ADR-0017** places `InternalError` in `web/src/error.rs` under an `ssr` feature with a flat `operator_message: String`, and calls the kind/class/context carrier "Forthcoming … tracked as jaunder-kq8w.16" — a dead bead-tracker id. The carrier landed and the type lives at `host/src/error.rs:94`. ADR-0059 explicitly extends 0017 and records the decision, so this is stale prose rather than an undecided reality.                                                                            | Task 18 |
| **ADR-0036's addendum** says the branch-protection ruleset requires PRs to be up to date with `main` so the gate runs against the merged tree. ADR-0077 supersedes exactly that: `strict_required_status_checks_policy` goes to `false` and the merge queue tests the combined state on a `gh-readonly-queue/…` branch. The view now cites 0077 alongside 0036. _(Not a defect in either ADR — 0077 states the supersession. Listed so task 18 can check whether 0036 wants a pointer.)_               | Task 18 |
| **ADR-0039 §3** calls `admin-site` "the lone global-singleton spec". The Playwright config now quarantines **two** — `admin-site` and `invite` — in the per-browser serial `*-admin` projects (`end2end/playwright.config.ts:72-105`).                                                                                                                                                                                                                                                                 | Task 18 |
| **ADR-0003 (`:17`, `:30-31`)** asserts user-uploadable stylesheets "remain architecturally distinct and are served from the storage layer". The feature was never built — no CSS handling in `storage/`, no config key, no path in `server/src`. An `accepted` ADR describing an unimplemented feature as though it shipped. Nearly deleted silently by the section pass; caught by verification.                                                                                                      | Task 18 |
| **ADR-0061 (`:51,:92,:97`)** names `Invalidator::patched`. No such method exists — `Invalidator` has only `new`/`notify`/`track`. The real symbol is a free function in another crate, `client::reactive::patched` (`client/src/reactive.rs:52`). The prose at `web/src/audiences/component.rs:48` repeats the stale name, so the drift is in the ADR _and_ in a code comment.                                                                                                                         | Task 18 |
| **ADR-0022 line 22** names `auth::generate_token` — zero hits anywhere in the tree, deleted by #458, surviving only in `docs/archive/`. Confirmed independently by two passes. Real drift inside an `accepted` ADR. Live equivalents: `host::token::generate_hashed` (session and reset tokens) and `host::invite::generate` (invite codes).                                                                                                                                                           | Task 18 |
| ~~**ADR-0089 §1** asserts a `deny.toml` removal that did not happen.~~ **WITHDRAWN.** Verified against git history, not just current state: `6328cb4d` (#199) did set `github = []`, executing ADR-0089 §1 in full; `0e8b66bb` (#297) later re-added it for the unrelated `lettre` patch. The ADR is correct. A lesson for the remaining sections: current state alone cannot distinguish "never done" from "done, then undone for another reason" — check the history before recording drift.         | —       |
| **ADR-0016 ~line 271** attributes component-SSR removal to #487, whose only commit is unrelated. See below.                                                                                                                                                                                                                                                                                                                                                                                            | Task 19 |

**Disposed without a new ADR** — evidence for the "the gap list shrinks under
scrutiny" expectation:

- Idempotency key → decided-elsewhere, issue #79, closed and shipped.
- Content-addressed media store → no longer a gap; ADR-0080, ADR-0084 and
  ADR-0090 have since decided the layout, the encoding, and reference semantics.
- Write-time stored rendering → decided in substance by ADR-0090 plus ADR-0079.
- RSD autodiscovery → noise; it is served, and a fixed discovery envelope needs
  no ADR. Promoted to body text.
- AtomPub categories document → **not served at all**; no route, no caller.
  Recorded as fact citing #928 (which mdorman had already opened independently).
- `rendered_html` "sanitized" doc-comment defect from the 2026-07 handoff →
  **retired**. ADR-0079 is built (`sanitize` establishes, `from_trusted`
  inherits, private field, static-check-pinned call site) and the code comment
  was corrected under #445.

### Code defects found by the doc work — file at task 16

Not doc problems. Stale comments in source, found while verifying the view. One
follow-up issue covers them all.

- `host/src/metrics.rs:4` — "Helper arguments are bounded enums". Not true of
  `atompub_request`, whose `op` is a matched-route-plus-method lookup.
- `xtask/src/server_fn_coverage/extract.rs:72,110,457` — three comments saying
  "`server-fn-tracing` writes `web.<vertical>.<ident>`". It no longer writes
  anything; #714 moved derivation into `#[macros::server]` and left the gate
  verify-only.
- `web/src/audiences/component.rs:48` — names `Invalidator::patched`, which does
  not exist (see the drift table).
- `elisp/jaunder-publish.el` — `jaunder--create-with-retry`'s docstring says it
  retries transport errors and 5xx, but the handler is a bare `(error …)`, so it
  retries any signalled error. A missing auth-source entry gets retried twice
  with sleeps before surfacing. Either narrow the handler or correct the
  docstring; the current behaviour is probably not intended.

### Known defects handed forward to later tasks

Found by a section pass but owned by another task. Do not fix them out of order;
the owning task must verify them itself.

- **Task 8 (Web frontend), around line 602.** The text asserts
  "`web::server_resource` (raw `Resource::new` is clippy-banned) is the only …".
  **`server_resource` does not exist in the source tree** — it survives only in
  `docs/adr/` and `docs/archive/`. It was deleted in #515 (commit `dd7baefb`).
  This is a live wrong claim about a symbol, the same class as the ADR-0023
  incident. Verify the clippy-ban half separately; it may still be true of
  something else.
- **Task 19 (tense correction), ADR-0016 line ~271.** The ADR attributes the
  removal of component SSR to **#487**, but #487's only code commit (`ead09db9`)
  models an edit-page route id as `Option` — nothing to do with SSR. Either the
  ADR's attribution is wrong or #487 is shorthand for work the commit log does
  not show. The view no longer repeats the number. Decide whether the ADR earns
  a past-tense correction note (metadata/navigation edits to an ADR are
  permitted) or whether this becomes a follow-up. Cross-check ADR-0041 and the
  #178/#180 CSR-cutover work before concluding.

### CONTEXT.md vocabulary worksheet

Accumulated by the section passes (brief step 6). This seeds the `CONTEXT.md`
backfill issue filed at task 16 — it is **not** edited in this branch. Append as
each section completes; do not deduplicate until task 16.

| Section   | Terms `CONTEXT.md` does not define                                                                                                                                                                                                                                                                                                                                                                                                                            |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Workspace | host crate; client crate; target-agnostic crate; wasm-only crate; proc-macro home; CSR performance mark                                                                                                                                                                                                                                                                                                                                                       |
| Storage   | `Backend` (marker trait — distinct from "backend", the deployment word); `Dialect` / per-trait `XDialect`; generic store `XStore<DB>`; composition root; `AtomicOps`; write-lock occupancy (hold duration vs acquisition count); write-first transaction; `BEGIN IMMEDIATE` / deferred-upgrade hazard; keyset cursor; shared ingestion layer and private user content layer (ADR-0006, unbuilt); idempotency key; backup target set; restore target emptiness |

### Inherited `un-ADR'd` flags by section

The 25 flags in the restored text, for the section passes to classify. Section
names as above; the parked text carried no flags in Workspace or the new
sections.

| Section             | Flags                                                                                                                                                                                                       |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Storage             | idempotency-key mechanism (migration `0023_create_idempotency_keys`; ADR-0047 names it only as follow-on #79)                                                                                               |
| Content model       | write-time stored rendering and local soft delete; the content-addressed media store (sha256 pathing), assumed by ADR-0024                                                                                  |
| Protocols           | surface-by-format matrix, HybridWindow selection, `feed_etag` conditional GET; the RSD autodiscovery and categories documents; publisher-side WebSub                                                        |
| Authentication      | cookie/Bearer/Basic precedence and `resolve_credential` homing; hashed-at-rest token storage; concrete cookie attributes; `cheap-kdf` and its dual fail-closed guard; the lowercase-canonical username rule |
| Web frontend        | the cargo-leptos-free wasm bundling pipeline                                                                                                                                                                |
| Observability       | `with_http_observability` request-id propagation; e2e VMs copying out `playwright-report-<backend>.json`                                                                                                    |
| Deployment          | the embedded-shell / on-disk-wasm split (#239); the CLI subcommand surface and `JAUNDER_BIND` / `JAUNDER_DB` / `JAUNDER_ENV`; the NixOS module and package outputs                                          |
| Emacs client        | service-document probe module; auth-source as credential store; client `Idempotency-Key` on create (#79, since built)                                                                                       |
| Verification gates  | the ladder also carries `sequence_check` and `host_tests`; `e2e-gate` also needs the CI elisp-integration job                                                                                               |
| Development tooling | ADR-0052 chartered 7 non-compiling checks, the set is now 8; the cargo-workspace exclusions (root `exclude`, the separate `tools/` workspace)                                                               |
| Docs and process    | ADR-0000 says transient docs are deleted; practice (#39) archives them as dated files                                                                                                                       |

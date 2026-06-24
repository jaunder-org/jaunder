# History Rebuild — Phase A (clean linear history) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a clean, green, **linear** history `L'` of the jaunder codebase — beads excised, messages rewritten, planning docs resurrected, do-then-redo collapsed, clean detours reordered — verified content-lossless against a frozen baseline, ending at a human review gate.

**Architecture:** Operate entirely on a throwaway clone (the real repo never moves). A `filter-repo` pre-pass strips excised paths; a generated `git rebase -i` todo drives a linear replay that drops/squashes/reorders commits, rewrites every message, and runs a per-commit green gate via `exec`. No merges are created in this phase — that is Phase B.

**Tech Stack:** `git` (2.54), `git filter-repo`, `cargo clippy`, `cargo fmt`, the project's `nextest`/e2e gates (run directly, *not* via `xtask`). Driver glue in shell/JS.

**Reference:** the approved design spec `docs/superpowers/specs/2026-06-22-history-rebuild-design.md` and the supporting analysis `HISTORY-REWRITE-SURVEY.md` (§8 decomposition, extractability, do-then-redo).

## Global Constraints

- **Lossless invariant (code):** after Phase A, every path *outside* `docs/`, `.beads/`, and `HISTORY-REWRITE-SURVEY.md` must be byte-identical to the frozen `pre-rebuild` baseline. The only sanctioned exceptions are the author's intentional edits at the review gate.
- **Per-commit green gate:** `cargo clippy` (workspace) + `cargo fmt --check`, run against **that commit's own tree/config** (never retrofitting later strictness).
- **Conventional commits:** subjects `type(scope): …`; scope ∈ `common · server · storage · web · e2e · build · docs · deps`.
- **No `Co-authored-by` trailers anywhere. No `chore(beads):` and no `jaunder-<id>` in any subject.**
- **Documentation lifecycle:** plan/spec docs retire `plans|specs/ → docs/archive/` at their deliverable's completion (the 30 dropped docs resurrected into this). **ADRs are exempt:** `docs/decisions/ → docs/adr/` (retroactive), never archived.
- **Message honesty:** rewritten bodies state only what is verifiable from the diff + referenced docs.
- All work happens in the clone at `$WORK` (set in Task 1). The real repo at `/home/mdorman/src/jaunder` is never modified.

---

## Tooling conventions

The executing agent has a **restricted Bash tool** and rich context-mode tooling. These apply to
*every* `Run:` block below — pick the surface by this rule, not by the fenced language:

- **JS / `node` → `ctx_execute(language: "javascript")`.** `node` is **blocked in the Bash tool**;
  every `node -e …` snippet here runs through context-mode (which has host filesystem + git access
  and persists writes to absolute host paths — that's how the bridge artifacts were generated).
- **Output processing (pipelines, log/blob scans, multi-step greps) → `ctx_execute(language:
  "shell")`.** Think-in-Code: process in the sandbox, print only the answer. `head` is also blocked
  in Bash.
- **Pass/fail gates (cargo, scripts) → `ctx_execute` with the BARE command** — no trailing
  `| tee` / `; echo $?` / `| head`, which replace the exit status and defeat `isError`.
- **File reads/edits → native Read / Edit / Grep**, not `cat`/`sed`/`grep` in Bash.
- **Bash is for git *state mutation* and short fixed observations only** (`git checkout`,
  `git commit`, `git rebase` continue/abort, `git status --short`, `git rev-parse`). The
  long-running, stateful rebase itself runs in Bash; its validations do not.

---

### Task 1: Working area + freeze baseline

**Files:** none in-repo (operates on a clone).

- [ ] **Step 1: Tag the freeze point on the real repo (lightweight, local).**

```bash
cd /home/mdorman/src/jaunder
git tag pre-rebuild main
git rev-parse pre-rebuild^{tree}   # record this; call it BASELINE_TREE
```

- [ ] **Step 2: Create the working clone.**

```bash
export WORK=/tmp/jaunder-rebuild
rm -rf "$WORK"
git clone --no-hardlinks --quiet /home/mdorman/src/jaunder "$WORK"
cd "$WORK" && git config user.name 'Michael Alan Dorman' && git config user.email 'mdorman@ironicdesign.com'
```

- [ ] **Step 3: Verify the clone matches the freeze.**

Run: `cd "$WORK" && [ "$(git rev-parse HEAD^{tree})" = "$BASELINE_TREE" ] && echo MATCH`
Expected: `MATCH`

---

### Task 2: `filter-repo` pre-pass (path excision + blob scan)

**Files:** removes `.beads/**` and `HISTORY-REWRITE-SURVEY.md` from all history.

- [ ] **Step 1: Blob-size report (flag, do not drop).** Run via `ctx_execute(shell)` (processing pipeline):

```bash
cd "$WORK"
git rev-list --objects --all | git cat-file --batch-check='%(objecttype) %(objectsize) %(rest)' \
  | awk '$1=="blob" && $2>262144 {print $2, $3}' | sort -rn | awk 'NR<=20'
```
Review the output with the author. Anything heavy and unwanted is added to the `--path` excise list below; otherwise proceed.

- [ ] **Step 2: Run the excision.**

```bash
cd "$WORK"
git filter-repo --force \
  --path .beads/ --path HISTORY-REWRITE-SURVEY.md --invert-paths
```
(`filter-repo` prunes now-empty commits by default.)

- [ ] **Step 3: Verify excision + prune + code-lossless.**

Run:
```bash
cd "$WORK"
echo "beads/survey paths remaining: $(git log --all --name-only --pretty=format: -- .beads HISTORY-REWRITE-SURVEY.md | grep -c .)"
echo "commit count: $(git rev-list --count HEAD)"   # expect ~40 fewer than baseline's 716 non-merge
git diff "pre-rebuild" HEAD -- . ':(exclude).beads' ':(exclude)HISTORY-REWRITE-SURVEY.md' ':(exclude)docs' --stat
```
Expected: `beads/survey paths remaining: 0`; commit count dropped ~40; the diff (outside docs/beads/survey) is **empty** — i.e., code unchanged. (`pre-rebuild` is fetched into the clone or referenced via the source repo.)

- [ ] **Step 4: Tag the pre-pass checkpoint.** `git tag prepass-done`

---

### Task 3: Deliverable catalog + commit→deliverable attribution

**Files:**
- Create: `$WORK/.rebuild/attribution.json`

**Interfaces:**
- Produces: `attribution.json` — an ordered array `[{sha, deliverable, kind}]` covering every surviving non-merge commit; `deliverable` is one of ~50 labels; `kind ∈ {feature, maintenance}`.

- [ ] **Step 1: Seed the deliverable catalog** from the plan/spec docs (the ~45-doc manifest in survey §8) plus the maintenance buckets (`build`, `coverage-tooling`, `docs`, `deps`) for cross-cutting work.

- [ ] **Step 2: Attribute every surviving commit.** For each commit, assign its deliverable using: (a) the keyword/thread map (survey §8), (b) hybrid homing — a feature's own tests/coverage/docs ride with it; genuinely cross-cutting commits go to a maintenance bucket. Persist to `attribution.json`.

- [ ] **Step 3: Verify coverage and consistency.** Run via `ctx_execute(language: "javascript")` (`node` is blocked in the Bash tool):
```bash
cd "$WORK"
node -e '
  const a=require("./.rebuild/attribution.json");
  const heads=require("child_process").execSync("git rev-list HEAD").toString().trim().split("\n");
  const assigned=new Set(a.map(x=>x.sha));
  console.log("commits:",heads.length,"assigned:",a.length,
    "missing:",heads.filter(h=>!assigned.has(h)).length,
    "deliverables:",new Set(a.map(x=>x.deliverable)).size);
'
```
Expected: `missing: 0`; `deliverables:` ~50.

- [ ] **Step 4: Commit the artifact.** `git add .rebuild/attribution.json && git commit -m "chore(rebuild): commit→deliverable attribution"` *(in the clone only; this commit is tooling scaffolding, dropped before Phase B reads the line.)*

---

### Task 4: Old→new message mapping

**Files:**
- Create: `$WORK/.rebuild/messages.json` — `{ sha: "<full new message>" }` for every surviving commit; squash-group leaders carry the composed message.

- [ ] **Step 1: Classify each message** as *already-informative* (preserve body, normalize subject) or *rough* (title-only / uninformative / bare doc-pointer → rewrite).

- [ ] **Step 2: Author messages** per spec §4.3: conventional subject; for rough commits author a body from the diff, **inlining context from the resurrected planning docs** rather than pointing at them; strip trailers; re-derive the 49 beads-referencing subjects from their diffs; honesty guardrail. Persist to `messages.json`.

- [ ] **Step 3: Verify the mapping mechanically.** Run via `ctx_execute(language: "javascript")` (`node` is blocked in the Bash tool):
```bash
cd "$WORK"
node -e '
  const m=require("./.rebuild/messages.json");
  const cc=/^(feat|fix|chore|docs|test|refactor|style|perf|build|ci|revert)(\([a-z]+\))?(!)?: /;
  const scopes=new Set(["common","server","storage","web","e2e","build","docs","deps"]);
  let bad=[];
  for(const [sha,msg] of Object.entries(m)){
    const subj=msg.split("\n")[0];
    if(!cc.test(subj)) bad.push([sha,"subject",subj]);
    const sc=(subj.match(/\(([a-z]+)\)/)||[])[1];
    if(sc&&!scopes.has(sc)) bad.push([sha,"scope",sc]);
    if(/co-authored-by/i.test(msg)) bad.push([sha,"coauthor"]);
    if(/chore\(beads\)|jaunder-[a-z0-9]/i.test(subj)) bad.push([sha,"beads",subj]);
  }
  console.log("violations:",bad.length); bad.slice(0,20).forEach(x=>console.log(" ",x.join(" | ")));
'
```
Expected: `violations: 0`.

- [ ] **Step 4: Commit the artifact.** `git add .rebuild/messages.json && git commit -m "chore(rebuild): old→new message mapping"`

---

### Task 5: Generate the Phase-A rebase todo

**Files:**
- Create: `$WORK/.rebuild/todo.txt` (rebase todo), `$WORK/.rebuild/seq-editor.sh`, `$WORK/.rebuild/msg-editor.sh`, `$WORK/.rebuild/gate.sh`.

**Interfaces:**
- Consumes: `attribution.json` (order), `messages.json`, the do-then-redo squash ranges and clean-detour reorder set (survey §4/§8).
- Produces: a todo where every surviving commit appears exactly once as `pick`/`squash`/`fixup`, with `exec .rebuild/gate.sh` after each commit boundary.

- [ ] **Step 1: Write `gate.sh`** (the per-commit green gate):

```bash
#!/usr/bin/env bash
set -e
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 2: Generate the todo** from the inputs: linear `pick` order = attribution order (A1); fold do-then-redo ranges into `squash`/`fixup`; move clean-detour blocks to their target positions; append `exec .rebuild/gate.sh` after each kept commit. Wire `seq-editor.sh` (installs the todo) and `msg-editor.sh` (supplies the `messages.json` entry per commit) via `GIT_SEQUENCE_EDITOR`/`GIT_EDITOR`.

- [ ] **Step 3: Verify the todo is total and well-formed.**

Run:
```bash
cd "$WORK"
echo "picks+squash: $(grep -cE '^(pick|squash|fixup)' .rebuild/todo.txt)"
echo "exec gates:   $(grep -c '^exec' .rebuild/todo.txt)"
```
Expected: pick+squash count == surviving-commit count; one `exec` per retained commit.

---

### Task 6: Run A1 — pure linearization (gated)

- [ ] **Step 1: Run the linear rebase (A1 ordering, no reorder/squash yet) with message rewrite + green gate.**

```bash
cd "$WORK"
GIT_SEQUENCE_EDITOR=.rebuild/seq-editor.sh GIT_EDITOR=.rebuild/msg-editor.sh \
  git rebase -i --root
```
If `gate.sh` halts the rebase, fix by reorder/squash of existing work (never net-new code), then `git rebase --continue`.

- [ ] **Step 2: Verify 0 conflicts + code-lossless.** (Step-0 measured 0; this is the gated repeat.)

Run:
```bash
cd "$WORK"
git diff pre-rebuild HEAD -- . ':(exclude).beads' ':(exclude)HISTORY-REWRITE-SURVEY.md' ':(exclude)docs' --stat
```
Expected: empty (code identical to baseline).

- [ ] **Step 3: Tag.** `git tag L0`

---

### Task 7: Run A2 — localized collapse + clean-detour reorder

- [ ] **Step 1: Apply each do-then-redo squash and each clean-detour reorder as an independent, regional `git rebase -i` on top of `L0`**, gate.sh running per commit. Each edit is local; a conflict halts only its region.

- [ ] **Step 2: Verify code-lossless still holds and history is still linear.**

Run:
```bash
cd "$WORK"
git diff pre-rebuild HEAD -- . ':(exclude).beads' ':(exclude)HISTORY-REWRITE-SURVEY.md' ':(exclude)docs' --stat
echo "merges: $(git rev-list --merges --count HEAD)"
```
Expected: empty diff; `merges: 0`.

- [ ] **Step 3: Tag.** `git tag L-cleaned`

---

### Task 8: Weave the documentation lifecycle

**Files:**
- Create: `docs/archive/**` (completed plan/spec docs, incl. the 30 resurrected).
- Rename: `docs/decisions/**` → `docs/adr/**` (permanent — never archived).

**Interfaces:**
- Consumes: `attribution.json` — each deliverable's *last* commit is its docs' retirement point.

- [ ] **Step 1: Build the doc → deliverable → retirement-commit map.** Each plan/spec doc retires at
  its deliverable's last commit; docs whose deliverable is still in-flight at HEAD never retire.

- [ ] **Step 2: Weave the moves into the linear history.** At each retirement commit, rename the
  deliverable's plan/spec doc(s) `plans|specs/ → archive/` — as a generated `docs: archive <X>`
  commit at the deliverable boundary (or folded into its last commit). Resurrect the 30 dropped docs
  into the lifecycle (content from `368a35fa^` per survey §7; they retire to `archive/` at their
  long-past completion). Relocate ADRs `docs/decisions/ → docs/adr/` retroactively; ADRs are
  **never** archived.

- [ ] **Step 3: Verify the lifecycle at HEAD and mid-history.**

Run:
```bash
cd "$WORK"
echo "archive docs:   $(git ls-files docs/archive | wc -l)"                                     # completed (≥30)
echo "active plans:   $(git ls-files docs/superpowers/plans docs/superpowers/specs | wc -l)"   # only in-flight
echo "adr dir:        $(git ls-files docs/adr | wc -l)"                                         # >0
echo "decisions gone: $(git ls-files docs/decisions | wc -l)"                                  # 0
# mid-history spot-check: for a completed deliverable's retirement commit R and its doc X:
#   git ls-tree -r R docs/archive | grep X            -> present
#   git ls-tree -r R docs/superpowers/plans | grep X  -> absent
```
Expected: archive ≥30; active = only in-flight (e.g. the emacs plan); adr >0; decisions 0; and the
mid-history spot-check shows X already archived by commit R.

---

### Task 9: Per-commit green sweep + tag `linear-clean`

- [ ] **Step 1: Independent green sweep** (confirms the `exec` gate, catches anything missed by A2/Task 8). Run via `ctx_execute(shell)` — long sweep; keeps per-commit output out of context:

```bash
cd "$WORK"
fail=0
for c in $(git rev-list --reverse HEAD); do
  git checkout --quiet "$c"
  cargo fmt --check >/dev/null 2>&1 && cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1 \
    || { echo "NOT GREEN: $c"; fail=$((fail+1)); }
done
git checkout --quiet L-cleaned
echo "non-green commits: $fail"
```
Expected: `non-green commits: 0`. (Long-running; this is the cost we accepted.)

- [ ] **Step 2: Tag.** `git tag linear-clean`

---

### Task 10: Review-gate packet (STOP)

- [ ] **Step 1: Produce the review packet** for the author:
  - `git log --oneline linear-clean` (the full rewritten linear story),
  - the code-lossless proof (empty diff vs `pre-rebuild` outside docs/beads/survey),
  - the doc-change summary (30 archived, ADRs relocated, beads/survey gone).

- [ ] **Step 2: HALT for review.** Phase B does **not** begin automatically. The author inspects `linear-clean`, may run their own `rebase -i` cleanup and make intentional content fixes (surfaced as an explicit `pre-rebuild..L'` code diff so intended ≠ accidental), and gives explicit go before Phase B.

---

## Self-Review

**Spec coverage:** §3 pre-pass → Task 2; §3 Phase A1 → Task 6; §3 A2 → Task 7; §3 review gate → Task 10; §4.1 beads excision → Task 2; §4.2 trailers / §4.3 messages → Task 4; §4.4 documentation lifecycle / §4.5 ADR exemption + move → Task 8; §4.6 do-then-redo / §4.7 detours → Task 7; §5 hybrid homing → Task 3; §6 per-commit gate → Tasks 6/9, lossless invariant → Tasks 2/6/7; §8 step 0 → already measured (Task 6 re-verifies). **Out of Phase-A scope (later plans):** §3 Phase B spine, §6 per-merge/e2e tiers, §7 cutover, §8 rehearsals A/B/C, §9 backlog migration.

**Placeholder scan:** Tasks 3 and 4 are judgment-data production — they specify the exact procedure *and* a mechanical validation gate rather than inlining ~50 attributions / ~670 messages (which are outputs, not plan text). All other tasks carry exact commands + expected output.

**Type consistency:** artifact names (`attribution.json`, `messages.json`) and tags (`pre-rebuild`, `prepass-done`, `L0`, `L-cleaned`, `linear-clean`) are used consistently across tasks.

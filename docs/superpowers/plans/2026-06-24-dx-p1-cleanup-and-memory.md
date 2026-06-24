# DX P1 — Cleanup & Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the dead `bd`/beads hooks from global Claude config and garden the project memory so it stops describing retired tooling (`bd`, `scripts/verify`, `check-coverage`, the old Projects).

**Architecture:** This is a **runbook**, not TDD — the deliverables are edits to global files under `~/.config/claude/` (config + memory markdown), none of which live in the jaunder repo. There is no compile/test cycle; verification is JSON-validity, a residual-token sweep, and the session-start memory load. Because these files are shared live by every running Claude instance, every edit is backup-first and atomic.

**Tech Stack:** JSON (Claude `settings.json`), Markdown (memory files), `node`/sandbox for validation, `rg` for the residual sweep.

## Global Constraints

- **Real config target:** `~/.config/claude/settings.json`. `~/.claude` is a **symlink** to `~/.config/claude` (verified: both paths share inode `62557767`). Edit the real target once; the symlink follows. Never edit through both paths.
- **Concurrency safety (from the spec):** a broken global config breaks every running instance. For each file: **back up first**, edit to a **complete valid file** in one atomic Edit, **validate** before moving on. Keep the backup until the whole task verifies.
- **No `Co-Authored-By` trailers** anywhere (overrides the global default). This is also one of the stale rules being removed from memory.
- **These files are untracked by git** — the worktree gives no safety net here; the explicit `.bak` backups are the rollback.
- **Memory-system rules:** update the entry that already covers a fact; delete entries that are now wholly wrong; do not append "correction" notes. `MEMORY.md` gets a one-line pointer per surviving memory and nothing else.

### Shared rewrite map (apply consistently wherever these tokens appear)

| Stale token | Replacement |
|-------------|-------------|
| `scripts/verify` (full gate) | `cargo xtask validate` |
| `scripts/verify --fast` | `cargo xtask check --no-test` |
| `scripts/check-coverage` / `check-coverage` | coverage now runs inside `cargo xtask check` / `validate` (Nix coverage check); the standalone script is legacy |
| `bd`/beads task tracking | GitHub Issues (`gh issue …`) in `jaunder-org/jaunder` |
| "use Sonnet subagents" | "use Opus subagents" |
| `Co-Authored-By` trailer guidance | remove (trailers are banned) |
| "Priority+Layer Projects" | "Projects: Jaunder Backlog / Operational Support / Privacy" |

---

## Task 1: Remove dead `bd` hooks from global settings (G6)

**Files:**
- Modify: `~/.config/claude/settings.json` (real target of the `~/.claude` symlink)

**Interfaces:**
- Consumes: nothing.
- Produces: a `settings.json` whose only change is the removal of the two `bd prime` hook invocations; all serena / context-mode hooks preserved byte-for-byte.

- [x] **Step 1: Confirm the symlink direction and back up the real file**

```bash
readlink -f ~/.claude            # expect: /home/mdorman/.config/claude
ls -li ~/.config/claude/settings.json ~/.claude/settings.json   # expect: identical inode
cp -p ~/.config/claude/settings.json ~/.config/claude/settings.json.bak
```
Expected: both paths show inode `62557767`; `.bak` created.

- [x] **Step 2: Remove the entire `PreCompact` block**

It contains only `bd prime`. Delete this whole top-level key (the trailing comma after the `}` belongs to `PreCompact`'s array close — remove it too so `PreToolUse` becomes the first key):

Remove:
```json
    "PreCompact": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "bd prime"
          }
        ]
      }
    ],
```
Result: `"hooks": {` is immediately followed by `"PreToolUse": [`.

- [x] **Step 3: Remove the `bd prime` hook object from the first `SessionStart` group**

Keep `serena-hooks activate`; remove only the `bd prime` object (and its trailing comma):

Change this:
```json
        "hooks": [
          {
            "type": "command",
            "command": "bd prime"
          },
          {
            "type": "command",
            "command": "serena-hooks activate --client=claude-code"
          }
        ]
```
to this:
```json
        "hooks": [
          {
            "type": "command",
            "command": "serena-hooks activate --client=claude-code"
          }
        ]
```

- [x] **Step 4: Validate the JSON is well-formed and bd-free**

```bash
node -e "const j=require('/home/mdorman/.config/claude/settings.json'); if(JSON.stringify(j).includes('bd prime')) throw new Error('bd prime still present'); if(!('PreToolUse' in j.hooks)||!('SessionStart' in j.hooks)||!('Stop' in j.hooks)) throw new Error('hook group missing'); if('PreCompact' in j.hooks) throw new Error('PreCompact not removed'); console.log('OK: valid JSON, no bd prime, serena+context-mode hooks intact');"
```
Expected: `OK: valid JSON, no bd prime, serena+context-mode hooks intact`.
(`node` is denied in the jaunder project deny-list but this is global config work; if blocked, run the same check via the sandbox `ctx_execute`.)

- [x] **Step 5: Remove the backup once validated**

```bash
rm ~/.config/claude/settings.json.bak
```
No git commit — this file is outside the repo.

---

## Task 2: Delete the wholly-stale memory files (G7a)

**Files:**
- Delete: `~/.config/claude/projects/-home-mdorman-src-jaunder/memory/feedback_memory_system_split.md`
- Delete: `~/.config/claude/projects/-home-mdorman-src-jaunder/memory/feedback_bd_show_batching.md`
- Delete: `~/.config/claude/projects/-home-mdorman-src-jaunder/memory/feedback_bd_graph_creation.md`

**Interfaces:**
- Produces: three fewer memory files; `MEMORY.md` index lines for them removed in Task 4.

- [x] **Step 1: Confirm each file is beads/RTK-only before deleting**

```bash
cd ~/.config/claude/projects/-home-mdorman-src-jaunder/memory
for f in feedback_memory_system_split.md feedback_bd_show_batching.md feedback_bd_graph_creation.md; do echo "== $f =="; rg -n "bd |beads|rtk" "$f" | wc -l; done
```
Expected: each is dominated by beads/RTK content (these encode `bd`-command mechanics and the beads-vs-memory split that no longer exists).

- [x] **Step 2: Delete the three files**

```bash
cd ~/.config/claude/projects/-home-mdorman-src-jaunder/memory
rm feedback_memory_system_split.md feedback_bd_show_batching.md feedback_bd_graph_creation.md
```

- [x] **Step 3: Verify deletion**

```bash
ls feedback_memory_system_split.md feedback_bd_show_batching.md feedback_bd_graph_creation.md 2>&1
```
Expected: three "No such file" messages.

---

## Task 3: Rewrite command-reference memories (G7b)

Apply the **Shared rewrite map**. Each file keeps its insight; only the dead tooling references change. Use Edit per occurrence; do not append correction notes.

**Files (Modify):** all under `~/.config/claude/projects/-home-mdorman-src-jaunder/memory/`
- `feedback_verify_before_commit.md` — `scripts/verify`→`cargo xtask validate`; drop any `bd`/`check-coverage` mention.
- `feedback_ctx_run_long_scripts.md` — retarget the example from `scripts/verify`/`check-coverage` to `cargo xtask check`/`validate`; keep the bare-command/`isError` guidance verbatim.
- `feedback_serena_contextmode_routing.md` — replace `scripts/verify`/`check-coverage` examples with `cargo xtask` ones; drop `bd` from the routing examples.
- `feedback_shell_tool_habits.md` — replace `scripts/verify`/`check-coverage` examples with `cargo xtask`; drop `bd`. (Keep all cd/grep/sed/Read/Edit guidance — it is the seed for DX P2.)
- `feedback_coverage_baseline_approval.md` — remove the "log a bead" phrasing; recast as "open a GH issue" if a tracker is referenced, else drop.
- `project_merge_conflict_resolution.md` — remove the `.beads/issues.jsonl` conflict line (no beads jsonl exists post-migration); keep coverage-manifest and `CONTRIBUTING.md` guidance; update any `scripts/verify`/`check-coverage` to `cargo xtask`.
- `project_dialect_files_no_infile_tests.md` — update the `check-coverage` mechanics to the `cargo xtask` Nix-coverage passes; the per-file-coverage insight stays.
- `project_target_dir_disk_bloat.md` — update `scripts/verify`/`check-coverage` references to `cargo xtask`; the `cargo clean` fix stays.

**Interfaces:**
- Produces: eight memories free of `scripts/verify`, `check-coverage`, and `bd` tokens (except `cargo xtask`).

- [x] **Step 1: Rewrite each file per the map** (Edit each occurrence; preserve frontmatter and the core insight).

- [x] **Step 2: Verify no residual dead tokens in these eight files**

```bash
cd ~/.config/claude/projects/-home-mdorman-src-jaunder/memory
rg -n "scripts/verify|check-coverage|\bbd \b|\bbeads\b" feedback_verify_before_commit.md feedback_ctx_run_long_scripts.md feedback_serena_contextmode_routing.md feedback_shell_tool_habits.md feedback_coverage_baseline_approval.md project_merge_conflict_resolution.md project_dialect_files_no_infile_tests.md project_target_dir_disk_bloat.md ; echo "exit=$?"
```
Expected: no matches (`exit=1`).

---

## Task 4: Overhaul `MEMORY.md` index + current-state (G7c)

**Files:**
- Modify: `~/.config/claude/projects/-home-mdorman-src-jaunder/memory/MEMORY.md`

**Interfaces:**
- Consumes: the file deletions (Task 2) and the surviving renamed/retargeted memories (Task 3).
- Produces: an index with no links to deleted files and no stale tooling/Projects wording.

- [x] **Step 1: Fix the current-state line** — replace the "26 beads → 24 issues w/ native types/deps/**Priority+Layer Projects** + 2 milestones" wording so the Projects read **Jaunder Backlog / Operational Support / Privacy** (per `project_history_rebuild_phaseB_done.md`, which is the accurate record). Keep the pointer to that file.

- [x] **Step 2: Remove the index lines** for the three deleted memories (memory-system split, bd show batching, bd graph creation) and for the autonomous-beads memory pending Task 5's outcome.

- [x] **Step 3: Update the "Workflow feedback" entries** whose one-line summaries cite `scripts/verify` (verify-before-commit, ctx-run-long-scripts) to the `cargo xtask` wording.

- [x] **Step 4: Verify the index has no dead links or stale tokens**

```bash
cd ~/.config/claude/projects/-home-mdorman-src-jaunder/memory
# every linked file in MEMORY.md must exist:
rg -o "\(([a-z0-9_]+\.md)\)" -r '$1' MEMORY.md | sort -u | while read f; do [ -f "$f" ] || echo "MISSING: $f"; done
rg -n "scripts/verify|check-coverage|Priority\+Layer|\bbd \b" MEMORY.md ; echo "tokens-exit=$?"
```
Expected: no `MISSING:` lines; `tokens-exit=1` (no stale tokens).

---

## Task 5: Decide the autonomous-work authorization memory (G7d) — USER-GATED

`feedback_autonomous_beads_authorization.md` is a **standing permission grant**, so it does not get silently rewritten. It is triply stale: beads-framed, says "use Sonnet subagents" (now Opus), and says "add the Co-Authored-By trailer" (now banned).

**Files:**
- Modify or Delete: `~/.config/claude/projects/-home-mdorman-src-jaunder/memory/feedback_autonomous_beads_authorization.md`

- [x] **Step 1: Get the user's choice** between:
  - **(a) Rewrite** — retarget to GitHub Issues, `cargo xtask validate` as the commit gate, Opus subagents, no trailer; keep the "kickoff-only on go / never main / never push" guardrails. Rename to `feedback_autonomous_work_authorization.md`.
  - **(b) Delete** — drop the standing authorization entirely; require explicit per-task go in future.

- [x] **Step 2: Apply the choice.** If (a): write the retargeted file, update the `MEMORY.md` index line to the new name. If (b): `rm` the file and remove its `MEMORY.md` index line.

- [x] **Step 3: Verify** the file state matches the choice and `MEMORY.md` has no dangling link to it.

---

## Task 6: Final residual-token sweep (G7 verification)

**Files:** none modified — verification only.

- [x] **Step 1: Sweep the whole memory dir for dead tooling tokens**

```bash
cd ~/.config/claude/projects/-home-mdorman-src-jaunder/memory
rg -n "scripts/verify|check-coverage|\brtk\b|Priority\+Layer" . ; echo "verify-exit=$?"
rg -n "\bbd \b|\bbeads\b|\.beads\b" . | rg -v "project_history_rebuild_phaseB_done.md" ; echo "beads-exit=$?"
```
Expected: `verify-exit=1` (none); the only surviving `bd`/`beads` mentions are the *historical* ones inside `project_history_rebuild_phaseB_done.md` (accurate record of the migration), so `beads-exit=1` after excluding it.

- [x] **Step 2: Confirm the global config is clean**

```bash
rg -n "bd prime" ~/.config/claude/settings.json ; echo "exit=$?"
```
Expected: no matches (`exit=1`).

---

## Self-Review

- **Spec coverage:** G6 → Task 1. G7 → Tasks 2 (deletes), 3 (rewrites), 4 (MEMORY.md), 5 (autonomous-auth, user-gated), 6 (sweep). Concurrency-safety constraint → backup/validate steps in Task 1 and the Global Constraints. No P1 spec item unmapped.
- **Placeholders:** none — every file is named, every token mapping is explicit, every check has an expected exit/output.
- **Consistency:** the Shared rewrite map is the single source for token replacements; Tasks 3–6 reference it rather than re-specifying. `node` denial caveat noted (use sandbox). Deletions in Task 2 are reflected in Task 4's index removals and Task 6's sweep exclusion list.

## Execution note

P1 mutates only global files; there is nothing to commit in the worktree except this plan doc itself. The worktree branch (`worktree-dx-program`) carries the spec + plans; P1's *effects* live in `~/.config/claude/`.

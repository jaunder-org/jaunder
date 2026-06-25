# DX Program — Developer-Experience Skills, Hooks & Config

- **Date:** 2026-06-24
- **Status:** Design approved; pre-implementation
- **Branch:** `reorg` (do not commit to `main`)
- **Author:** brainstormed with Claude from transcript evidence

## Problem

Recurring development friction keeps costing turns and re-corrections. The same
mistakes recur even though many are already captured as personal-memory
feedback — passive recall has demonstrably failed on the mechanical ones. This
program turns the highest-frequency, lowest-effort fixes into the lever that
actually *enforces* them (hooks/config) or reliably *prompts* them (skills),
instead of relying on memory.

## Evidence

Mined from 308 session transcripts (157 MB) plus the tool-error stream, in the
sandbox (raw bytes never entered context). Counts are occurrences across the
corpus:

- **`cd …` — 763×**: redundant `cd <project-root> && …`, my single most common
  Bash shape. The shell cwd already persists between calls.
- **`… && echo` — 491×**: echo-after-command "did it really work" tic; the exit
  code / `isError` already carries that signal.
- **Permission deny-list dead-loops — ~110×**: `grep` (43), `head` (40),
  `tail` (26), `sed` (15), `cd` (8) are denied, and I retried them anyway. The
  denial is a bare "security policy" error with no redirect.
- **Serena symbolic-tool avoidance — ~130×**: "Too many consecutive
  read/grep calls without using symbolic tools" (98 + 22 + 10).
- **Edit hygiene — ~73×**: "File has not been read yet" (51), "modified since
  read" (12), "String to replace not found" (10).
- **`scripts/verify 2>&1` — 69×**: long scripts run under Bash with redirection
  instead of routed through context-mode for pass/fail gating.
- **Verbal corrections** confirming the above: verify-before-commit and
  exit-code-blindness (#3, #4, #44, #6), context-mode routing (#7, #9, #14,
  #36, #47), `cd`/`sed`/`cat` (#13, #18, #22, #25, #36, #46, #48), opus
  subagents + plan-item checkoff (#20, #26, #32, #39), boilerplate/helpers and
  `AppState` drift (#5, #42).

## Goals / Non-goals

In scope (three layers, confirmed):

1. **Tool-driving behavior** — how the shell/editor is driven.
2. **Workflow procedures** — multi-step judgment skills (jaunder-specific).
3. **Permission / env config** — the deny-list and dead config.

Out of scope:

- **Code-quality / DRY** (helpers-over-boilerplate, keep `AppState`
  storage-only). Real (#5, #42) but **deferred** — parked as a GH issue, to be
  picked up after this program lands. Tackle interactively via `/code-review`
  or `/simplify` in the meantime.

## Principles

- **Redirect, don't deny.** Every block names the sanctioned alternative
  (`rg`, not `grep`, because it respects `.gitignore`; Read, not `head`/`cat`;
  Edit, not `sed`; context-mode for processing). A bare denial causes baroque
  workarounds and retry dead-loops; a named redirect does not. Where it is
  safe, **rewrite** (strip the redundant `cd` and run the rest) rather than
  block the whole command.
- **Enforce vs remind — pick the lever by fit.** Mechanical tics where recall
  has failed → hooks/config (deterministic). Branching judgment → skills.
- **Split by nature.** Universal fixes live in global config
  (`~/.config/claude/`); jaunder-specific procedures live in the jaunder repo
  (`.claude/`, `docs/`).

## Existing infrastructure (verified 2026-06-24)

- `rtk` is **gone** — no settings file references it (the `RTK.md` doc lingers
  in global `CLAUDE.md` but no hook calls it).
- **Global** `~/.config/claude/settings.json` (symlinked to `~/.claude/settings.json`
  on purpose, so self-installing tools land correctly — *not* a smell):
  PreToolUse `serena-hooks remind` + `serena-hooks auto-approve`; PreCompact &
  SessionStart `bd prime` (**dead — beads retired**); SessionStart serena
  activate + `context-mode-cache-heal.mjs`; Stop serena cleanup.
- **Project** `jaunder/.claude/settings.local.json`: PreToolUse
  `deny-bash-script-runners.mjs` (extension point); PostToolUse `scripts/git-add`;
  permissions = 41 allow, 7 deny (`sed`, `grep`, `head`, `tail`, `node`,
  `cd /home/mdorman/src/jaunder[/]`).
- Current verification entrypoints (the dead `scripts/verify` is replaced):
  - `cargo xtask check` — inner loop, auto-fixes fmt, host static + clippy +
    Nix coverage (`--no-test` = static + clippy only).
  - `cargo xtask validate` — full gate, never mutates tree, + e2e VMs
    (`--no-e2e` to skip).
  - `scripts/check-coverage` is legacy; coverage now runs via xtask's Nix check.

## Item set

### Global (universal — `~/.config/claude/`)

| ID | Item | Lever | Evidence |
|----|------|-------|----------|
| G1 | Strip redundant leading `cd <project-root> &&`, run the rest | global hook | `cd …` 763× |
| G2 | Deny → named-tool redirect: `grep`→`rg`, `head`/`tail`/`cat`→Read, `sed`→Edit, processing→context-mode | global hook + message | ~110 dead-loops |
| G3 | Flag `&& echo` / `; echo $?` tails — trust exit code / `isError` | global hook (warn) | `… && echo` 491× |
| G4 | Serena-first routing — prefer `find_symbol`/`get_symbols_overview` over raw Read/grep sprees | guidance (judgment) | ~130 nudges |
| G5 | Edit read-first hygiene — Read before Edit; re-read if stale | guidance (judgment) | ~73 errors |
| G6 | Remove dead `bd prime` hooks (PreCompact + SessionStart) | global config | beads retired |
| G7 | Memory-gardening pass — audit every `MEMORY.md` entry and `feedback_*`/`project_*` file; delete beads-only memories, rewrite `scripts/verify`→`cargo xtask`, fix the old Projects (Priority/Layer → Privacy / Operational Support / Backlog) and stale current-state | global config (memory dir) | stale refs found 2026-06-24 |
| G8 | Worktree-workflow guidance — default to git worktrees for most dev; reactivate Serena for the worktree path after entering one (cwd moves, Serena stays on the old root) | memory (feedback) | user request 2026-06-24 |

G4/G5 are guidance-lever and honestly cannot be hook-enforced; a skill can
prompt the habit but not guarantee it.

### Jaunder-specific (`jaunder/.claude/`)

| ID | Item | Lever | Evidence |
|----|------|-------|----------|
| J1 | commit/ship skill — run the current gate (`cargo xtask validate`, or `check` for inner loop) → check exit code → then commit | project skill | #3, #4, #44, #6 |
| J2 | subagent-dispatch skill — opus model, plan-item checkoff, `cargo xtask check --no-test` per-task gate, e2e timeout guidance (seeds from issue #5) | project skill | #5, #20, #26, #32, #39 |
| J3 | context-mode routing for long scripts — run the gate bare under context-mode for reliable pass/fail (no `2>&1` / `\|head` that defeat `isError`) | project skill / extend deny-redirect | `verify 2>&1` 69× |
| J4 | project deny-list → strip/redirect (extend `deny-bash-script-runners.mjs`) so denied tools redirect instead of dead-blocking | project hook | the 7 denies |

## Tracking & structure

- **Single source of truth: this spec.** It carries the whole program (global +
  project) as the master plan. Global items (G1–G6) have *no* GitHub issue —
  global config is not a git repo, and they are deliberately kept out of the
  jaunder project. They are tracked here only. **No** `~/.config/claude/DX-TODO.md`
  breadcrumb (decided against).
- **GitHub, jaunder-trackable slice only:**
  - New **"Developer Experience"** Project, parallel to Privacy / Operational
    Support.
  - Issues: **J1, J2, J3, J4**, all `dx`-labelled. **Issue #5 stays as-is** and
    joins the project (it is J2's seed). Plus one **parked DRY issue**
    (deferred), added in a "later" state.

## Sequencing

1. **G6 + G7** — rip out dead `bd` hooks, then garden the memories (do this
   early: stale memories actively mislead the later implementation work).
2. **G1–G3** — cd-strip / deny-redirect / echo, one global hook (the
   763× / 491× / ~110× wins).
3. **J4 + J1 + J3** — project deny-redirect + commit/ship + ctx routing
   (built on the confirmed `cargo xtask` entrypoints).
4. **J2** — subagent dispatch; then **G4 / G5** guidance.
5. **DRY** — deferred.

## Concurrency safety (live global-config edits)

The worktree isolates *repo* files, but G1–G3, G6, and G7 edit **global** config
and memory under `~/.config/claude/` — shared, live, by every running Claude
instance. A broken global PreToolUse hook breaks *every* instance on its next
tool call. All global edits MUST follow:

- **Fail-open hooks.** The redirect/strip hook must treat any internal error as
  "allow unchanged" (exit 0, no block) and never hang. A guard-rail that breaks
  tool use is worse than the friction it prevents.
- **Build inert, activate last.** Develop and fully test each hook script as a
  standalone file first; the `settings.json` edit that *wires it in* is the
  final, smallest, atomic step — never the first.
- **Test before activation.** Exercise the hook script directly against sample
  tool-call JSON (valid input, malformed input, unrelated tools) and prove it
  always emits valid output and exits 0, *before* any global settings reference
  it.
- **Atomic, validated settings writes.** Never leave `settings.json` partially
  written. Write a complete, JSON-validated file in one step (or temp-write +
  rename); the file is symlinked, so edit the real target once and verify the
  link first.
- **Backup + reversible.** Snapshot `settings.json` (and any memory file)
  before editing so a bad change reverts instantly.
- **Activation timing.** The settings edit takes effect for other instances on
  their next tool call. Apply global activations when the user has quiesced
  other instances, or rely on the fail-open guarantee — per the user's choice at
  implementation time.
- **Memory edits are low-risk** for *running* instances (already loaded) but
  affect new sessions; still write atomically.

## Implementation findings (2026-06-24, from P2 research)

- **Claude Code v2.1.x PreToolUse hooks cannot rewrite a Bash command** (`updatedInput.command` is ignored for Bash; verified via context-mode `formatters.mjs`). So **G1 can't *strip* the `cd`** — it can only deny or nudge. User chose a **non-blocking `additionalContext` nudge** for G1 (no dead-loop, no baroque workaround).
- **G3 (echo)** likewise becomes a non-blocking nudge.
- **G2 (deny-redirect) is deferred out of P2.** Making its guidance show requires moving `grep`/`head`/`tail`/`sed` out of `permissions.deny`, but context-mode **mirrors `permissions.deny`**, so that would re-open those tools on context-mode's shell (a PreToolUse hook only gates the Bash tool). G2 is therefore folded into a later plan **with J4**, where the trade-off is decided deliberately.
- **Revised plan slicing:** P1 (cleanup+memory, done) → **P2 = G1+G3 nudge hook** → later: G2+J4 (deny-redirect + project deny-list) → J1/J3 skills → J2 dispatch → DX Project/issues. Hooks load at session start, so wired hooks are live-verified only after a restart.

## Open implementation notes

- Locate and integrate with the existing hook chain (`serena-hooks` global;
  `deny-bash-script-runners.mjs` project) rather than assuming greenfield; the
  new global redirect hook must coexist with `serena-hooks` on PreToolUse.
- The global settings file is symlinked — edit once; verify the symlink target
  before writing.
- Confirm the exact `bd prime` hook entries to remove without disturbing the
  adjacent serena / context-mode hooks in the same PreCompact/SessionStart
  groups.
- G7 gardening: follow the memory-system rules (update the entry that already
  covers a fact; delete memories that are now wrong) rather than appending
  corrections. Known stale: `feedback_verify_before_commit`,
  `feedback_autonomous_beads_authorization`, `feedback_ctx_run_long_scripts`
  (all cite `scripts/verify`); the beads-only files
  (`feedback_memory_system_split`, `feedback_bd_show_batching`,
  `feedback_bd_graph_creation`, and the beads half of others); `MEMORY.md`
  current-state and the Projects names.

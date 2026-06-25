# DX P4 — `jaunder-ship` Skill (J1, with J3 folded in) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:writing-skills to author the skill, then superpowers:executing-plans / subagent-driven-development to run this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A personal `jaunder-ship` skill that codifies the commit procedure — run the `cargo xtask` gate through context-mode, believe the exit code, make one clean commit — so commits stop landing on unverified or misread checks.

**Architecture:** A single personal skill at `~/.config/claude/skills/jaunder-ship/SKILL.md` (Claude Code skill = a directory with `SKILL.md`, YAML frontmatter `name`/`description` + markdown body). Personal/untracked (decided: it leans on context-mode routing other contributors lack), scoped to jaunder via its description. J3 ("run long scripts via context-mode, gate on exit code") is NOT a separate skill — its commit-relevant part is folded into step 2/3 here; the general habit stays in the `feedback_ctx_run_long_scripts` memory.

**Tech Stack:** Markdown skill file; YAML frontmatter. Authoring follows the superpowers:writing-skills skill.

## Global Constraints

- **Personal, not committed to the repo:** the skill file lives in `~/.config/claude/skills/` (outside the repo). The committed artifact is this plan.
- **Scoped by description:** the description must make clear it applies to the jaunder repo, so it triggers there and not elsewhere.
- **Single source for facts it cites:** `cargo xtask validate` (full gate) / `cargo xtask check --no-test` (inner loop), per `feedback_verify_before_commit`. No `Co-Authored-By`. Never commit to `main`.
- **Skills load at session start** — discoverability is verified in a fresh session; frontmatter validity is verified now.

---

## Task 1: Author the `jaunder-ship` skill

**Files:**
- Create: `~/.config/claude/skills/jaunder-ship/SKILL.md`

**Interfaces:**
- Produces: an invocable `jaunder-ship` skill whose checklist drives the commit procedure.

- [ ] **Step 1: Invoke superpowers:writing-skills** to author the skill (it governs skill structure, naming, and the verification expectations).

- [ ] **Step 2: Write `~/.config/claude/skills/jaunder-ship/SKILL.md`** with exactly this content:

```markdown
---
name: jaunder-ship
description: "Use when about to commit changes in the jaunder repo — runs the cargo xtask gate through context-mode, checks the exit code, then makes one clean commit. Invoke before every jaunder commit."
---

# Jaunder Ship — verify then commit

Use this whenever you are about to commit in the jaunder repo. It exists because
commits have repeatedly landed on an unverified or partial check (e.g. a
single-crate `nextest`), and because the gate's pass/fail was misread (exit code
ignored).

## Checklist

1. **Not on main.** Confirm the branch is not `main`/`master`
   (`git branch --show-current`). Never commit to main without explicit user consent.
2. **Run the gate through context-mode, bare.** The commit gate is
   `cargo xtask validate` (static + clippy + Nix coverage + e2e VMs). For the
   inner loop while fixing, use `cargo xtask check --no-test`.
   - `ctx_execute(language: "shell", code: "cargo xtask validate", intent: "failing step, clippy errors, coverage regression, e2e failures, final exit code")`
   - Run the BARE command — no trailing `; echo $?`, `| tee`, `| head`, `| rg`:
     they replace the exit status and defeat `isError`. (See the
     ctx-run-long-scripts memory.)
3. **Believe the exit code, not a glance.** If the call is flagged `isError`
   (non-zero exit), the gate FAILED — fix the cause and re-run; do NOT commit.
   Only proceed on a clean pass. Don't hand-run `cargo build`/`clippy`/`nextest`
   to "diagnose" — re-run the gate.
4. **One clean commit.** Stage the change and make a single commit for the task
   (avoid follow-up fixups). NO `Co-Authored-By` trailer.
5. **Don't push.** Branches are local; merges to main are local.
6. **Verifying a subagent's work?** Re-run the gate yourself before trusting
   "DONE" — subagents over-report. Inspect the actual git diff for ignored
   DO-NOT-COMMIT instructions and duplicated helpers.
```

- [ ] **Step 3: Validate the frontmatter** (sandbox — parse the YAML header, confirm `name` matches the directory and a non-empty `description`):

```javascript
const fs = require('fs');
const p = '/home/mdorman/.config/claude/skills/jaunder-ship/SKILL.md';
const text = fs.readFileSync(p, 'utf8');
const fm = text.match(/^---\n([\s\S]*?)\n---/);
console.log('has frontmatter: ' + !!fm);
const name = (text.match(/^name:\s*(.+)$/m) || [])[1];
const desc = (text.match(/^description:\s*(.+)$/m) || [])[1];
console.log('name: ' + name + ' (matches dir: ' + (name === 'jaunder-ship') + ')');
console.log('description scoped to jaunder: ' + /jaunder/i.test(desc || ''));
console.log('mentions cargo xtask: ' + /cargo xtask/.test(text));
console.log('no Co-Authored-By leak: ' + !/Co-Authored-By:/i.test(text.replace(/NO `Co-Authored-By`[^\n]*/i,'')));
```
Expected: has frontmatter true; name matches dir; description scoped to jaunder; mentions cargo xtask.

- [ ] **Step 4: Record.** No repo commit (personal file). Note: live discoverability (the skill appearing in the available-skills list and triggering at commit time) is confirmed in the next fresh session, since skills load at session start.

---

## Self-Review

- **Spec coverage:** J1 (commit/ship: verify → exit code → one clean commit) → the checklist. J3's commit-relevant part (ctx routing + bare command + `isError`) → steps 2–3; the general J3 habit remains in `feedback_ctx_run_long_scripts` (no duplicate skill). Personal-location decision honored.
- **Placeholders:** none — the full SKILL.md content is inline; the validation has expected output.
- **Consistency:** commands (`cargo xtask validate` / `check --no-test`) match `feedback_verify_before_commit` and the P1 gardening; "no Co-Authored-By" and "never main" match standing prefs.

## Execution note

Personal skill file only; committed artifact is this plan. Mark steps `- [x]` and commit the plan, as prior plans did.

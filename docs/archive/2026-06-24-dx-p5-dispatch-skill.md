# DX P5 — `jaunder-dispatch` Skill (J2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:writing-skills to author, then executing-plans / subagent-driven-development to run. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A personal `jaunder-dispatch` skill holding the canonical brief for delegating jaunder implementation work to a subagent — so the ~40-line guidance block stops being re-pasted/reconstructed each time and subagents stop reverse-engineering the gate (issue #5).

**Architecture:** Personal skill `~/.config/claude/skills/jaunder-dispatch/SKILL.md`, scoped to jaunder by description. It is a reference + technique skill: a paste-ready subagent brief plus controller-side verification. It complements (does not replace) `superpowers:subagent-driven-development`, which governs orchestration. RED evidence = the documented failures in issue #5 (re-pasted guidance; subagents reverse-engineered the coverage gate and SSR resource race; one e2e run timed out and confused a subagent).

**Tech Stack:** Markdown skill; authoring follows superpowers:writing-skills.

## Global Constraints

- **Personal/untracked**, `~/.config/claude/skills/` (decided in P4). Committed artifact = this plan.
- **Single source for facts:** `cargo xtask check --no-test` = per-task subagent gate; `cargo xtask validate` = controller's final gate (per `feedback_subagent_verify_with_xtask_check`). Opus subagents (per MEMORY). New-file persistence via `ctx_execute` `fs.writeFileSync` (per `feedback_subagent_writes_via_ctx_execute`).
- **Description = triggers only**, no workflow summary (writing-skills SDO rule).

---

## Task 1: Author the `jaunder-dispatch` skill

**Files:**
- Create: `~/.config/claude/skills/jaunder-dispatch/SKILL.md`

- [x] **Step 1: Invoke superpowers:writing-skills** to author it.

- [x] **Step 2: Write `~/.config/claude/skills/jaunder-dispatch/SKILL.md`** with exactly:

```markdown
---
name: jaunder-dispatch
description: "Use when delegating implementation work in the jaunder repo to a subagent — handing off a plan task or sub-task, or assembling the prompt for a subagent that will touch jaunder code."
---

# Jaunder Subagent Dispatch

Use when handing a jaunder implementation task to a subagent. Subagents don't
read CONTRIBUTING/ADRs unless told, so they reverse-engineer the gate and waste
turns. This is the canonical brief so you don't reconstruct it each time.
Orchestrate with `superpowers:subagent-driven-development`; this skill is the
jaunder-specific content to include.

## Dispatch brief — paste into the subagent's prompt

- **Model: Opus.** (Opus subagents need less fixup on return.)
- **Orient, don't reverse-engineer.** Read `CONTRIBUTING.md` (testing, the gate,
  coverage policy) and any ADR / `web-style-guide.md` the task touches before
  coding. The gate and coverage rules are documented — don't infer them.
- **Per-task gate: `cargo xtask check --no-test`** (clippy + fmt + static). Do
  NOT hand-run `cargo build`/`nextest`/`clippy` ad hoc. Do NOT run the full
  `cargo xtask validate` (coverage + e2e VMs) — that is the controller's final
  gate.
- **e2e:** don't run the e2e VMs; they are slow and a timeout will confuse you.
  Leave e2e to the controller's `validate`.
- **New files:** when running in the background you can't Write/Bash new files
  (auto-denied, no interactive approval). Persist a new file via `ctx_execute`
  `fs.writeFileSync` to its absolute host path, then self-verify on disk.
- **Check off plan items** in the plan document (`- [ ]` → `- [x]`) as you
  complete them.
- **Report honestly.** Don't claim DONE unless `cargo xtask check --no-test`
  passed clean; quote the real result. Don't commit unless told.

## Controller side (you, dispatching)

- Dispatch via `superpowers:subagent-driven-development`; one task per subagent.
- **Verify every "DONE" yourself** — re-run the gate (see the `jaunder-ship`
  skill) and read the diff. Subagents over-report (one reported 1058 tests
  passing on code that didn't compile). Watch for ignored DO-NOT-COMMIT
  instructions and duplicated helpers.
- Use Opus for the subagents.
```

- [x] **Step 3: Validate frontmatter + content** (sandbox)

```javascript
const fs=require('fs');
const t=fs.readFileSync('/home/mdorman/.config/claude/skills/jaunder-dispatch/SKILL.md','utf8');
const name=(t.match(/^name:\s*(.+)$/m)||[])[1];
const desc=(t.match(/^description:\s*(.+)$/m)||[])[1];
console.log('name matches dir: '+(name==='jaunder-dispatch'));
console.log('name charset ok: '+/^[a-z0-9-]+$/.test(name||''));
console.log('desc starts "Use when": '+/^"?Use when/.test(desc||''));
console.log('desc scoped to jaunder: '+/jaunder/i.test(desc||''));
console.log('desc has NO workflow summary (no "paste"/"gate"/"opus"): '+!/(paste|gate|opus|checklist)/i.test(desc||''));
console.log('cites check --no-test: '+/cargo xtask check --no-test/.test(t));
console.log('cites subagent-driven-development: '+/subagent-driven-development/.test(t));
console.log('word count: '+t.split(/\s+/).filter(Boolean).length+' (<500)');
```
Expected: all true; word count < 500.

- [x] **Step 4: Record.** No repo commit (personal file). Live discoverability confirmed when the skill appears in the available-skills list (skills load in-session, observed with `jaunder-ship`).

---

## Self-Review

- **Spec coverage:** J2 (opus, plan-checkoff, `cargo xtask check --no-test` gate, e2e-timeout, doc-pointers) → the brief; new-file ctx-write + controller verify folded in. Issue #5's "re-pasted block / reverse-engineering / e2e timeout" → addressed directly.
- **Placeholders:** none — full SKILL.md inline, validation has expected output.
- **Consistency:** gate commands match `feedback_verify_before_commit` / `feedback_subagent_verify_with_xtask_check` / `jaunder-ship`; defers orchestration to `subagent-driven-development` rather than duplicating it.

## Execution note

Personal skill file; committed artifact is this plan. Mark steps `- [x]` and commit the plan, as prior plans did.

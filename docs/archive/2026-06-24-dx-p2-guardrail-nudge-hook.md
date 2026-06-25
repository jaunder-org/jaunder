# DX P2 — Global Guard-Rail Nudge Hook (G1 + G3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A global PreToolUse Bash hook that emits non-blocking `additionalContext` nudges when a command has a redundant leading `cd <project-root> &&` (G1) or a trailing confirmation `echo` (G3) — retraining the two highest-frequency tics (763× / 491×) deterministically, without ever blocking a command.

**Architecture:** One Node ESM file at `~/.config/claude/hooks/bash-nudges.mjs`. Its logic is two **pure exported functions** — `analyze(command, cwd) → string[]` and `formatOutput(nudges) → string|null` — plus a thin stdin/stdout glue guarded so it runs only when executed as the hook (not when imported by a test). The hook is **inert** until referenced from `settings.json`; activation (the settings edit) is the final atomic step. It only ever returns `additionalContext` (never a deny/ask decision), so it cannot block a tool call or, we expect, short-circuit the other PreToolUse hooks.

**Tech Stack:** Node ESM (`.mjs`), `node:path`/`node:os`/`node:url`. Tests run in the context-mode JS sandbox via dynamic `import()` (the `node` *CLI* is denied on both Bash and context-mode shells, so we exercise the exported functions directly, not by spawning the script).

## Global Constraints

- **Scope is decided:** G1 and G3 only, both **non-blocking nudges**. G2 (deny-redirect) is OUT of P2 — deferred to a later plan with J4 because of the `permissions.deny`↔context-mode coupling.
- **Fail-open (from the spec):** any malformed/unexpected input → no output, `process.exit(0)`, command proceeds unchanged. A non-zero exit also fails open (Claude ignores the hook), but exit 0 is the only intended path.
- **`additionalContext` only** — never emit `permissionDecision`. This keeps it non-blocking and avoids the "first decision wins" short-circuit that could suppress the project `deny-bash-script-runners.mjs` hook.
- **Build inert, activate last:** write + fully test the hook file before adding it to `settings.json`. The settings edit is backup-first, atomic, JSON-validated (the real config target is `~/.config/claude/settings.json`; `~/.claude` is a symlink to it — edit the real target once).
- **Hooks load at session start:** the wired hook cannot be live-tested in the authoring session. Standalone sandbox tests prove correctness; live behavior is verified in the next fresh session.
- **No `Co-Authored-By` trailers.** Worktree branch `worktree-dx-program`, never `main`.
- **The hook file is outside the repo** (global config, not a git repo) — no commit for the hook itself; the committed artifact is this plan. Sandbox tests are the verification of record.

---

## Task 1: Pure `analyze` / `formatOutput` core + unit tests

**Files:**
- Create: `~/.config/claude/hooks/bash-nudges.mjs` (functions only this task; glue added in Task 2)

**Interfaces:**
- Produces: `export function analyze(command, cwd) → string[]` and `export function formatOutput(nudges) → string|null`, consumed by Task 2's glue and by the sandbox tests.

- [x] **Step 1: Write the failing test** (sandbox, dynamic import of the not-yet-written functions)

Run via `ctx_execute(language: "javascript", code: <below>)`:
```javascript
const m = await import('/home/mdorman/.config/claude/hooks/bash-nudges.mjs');
const CWD = '/home/mdorman/src/jaunder';
const cases = [
  ['cd /home/mdorman/src/jaunder && cargo build', CWD, 1, 'cd'],
  ['cd /home/mdorman/src/jaunder/ && cargo build', CWD, 1, 'cd-trailing-slash'],
  ['cd end2end && npm test', CWD, 0, 'subdir-cd-ok'],
  ['cargo build && echo done', CWD, 1, 'echo'],
  ['cargo build; echo $?', CWD, 1, 'echo-status'],
  ['cd /home/mdorman/src/jaunder && cargo build && echo ok', CWD, 2, 'both'],
  ['cargo build', CWD, 0, 'clean'],
  ['echo hello', CWD, 0, 'leading-echo-ok'],
  ['', CWD, 0, 'empty'],
  [null, CWD, 0, 'null'],
];
let fail = 0;
for (const [cmd, cwd, want, label] of cases) {
  const got = m.analyze(cmd, cwd).length;
  const ok = got === want;
  if (!ok) fail++;
  console.log((ok ? 'PASS ' : 'FAIL ') + label + ' (want ' + want + ', got ' + got + ')');
}
console.log(m.formatOutput([]) === null ? 'PASS formatOutput([])=null' : 'FAIL formatOutput([])');
console.log(/additionalContext/.test(m.formatOutput(['x']) || '') ? 'PASS formatOutput(["x"])' : 'FAIL formatOutput(["x"])');
console.log(fail === 0 ? '\nALL PASS' : '\n' + fail + ' FAILURES');
```
Expected now: import throws (file/exports don't exist) → FAIL.

- [x] **Step 2: Write the implementation** (create the file with functions only)

```javascript
import path from "node:path";
import os from "node:os";

// Returns an array of nudge strings for the given command. Pure; never throws on
// bad input (returns []). `cwd` is the working directory the command runs in.
export function analyze(command, cwd) {
  const nudges = [];
  if (typeof command !== "string" || !command.trim()) return nudges;

  // G1: redundant leading `cd <target> (&&|;)` where <target> resolves to cwd.
  const cd = command.match(/^\s*cd\s+(['"]?)([^'"&;|]+)\1\s*(?:&&|;)/);
  if (cd && cwd) {
    let target = cd[2].trim();
    if (target === "~" || target.startsWith("~/")) {
      target = path.join(os.homedir(), target.slice(1));
    }
    if (path.resolve(cwd, target) === path.resolve(cwd)) {
      nudges.push(
        "Drop the leading `cd " + cd[2].trim() + " &&` — the Bash working directory already " +
        "persists at the project root, and prefixing `cd` turns the command into a compound your " +
        "allowlist can't match (so it prompts). Run the command bare."
      );
    }
  }

  // G3: trailing confirmation echo, e.g. `... && echo done` / `... ; echo $?`.
  if (/(?:&&|;)\s*echo\b[^&;|]*$/.test(command)) {
    nudges.push(
      "Drop the trailing `echo` — the tool's exit code / `isError` already tells you whether the " +
      "command succeeded; you don't need an echo marker to confirm it ran."
    );
  }

  return nudges;
}

// Builds the PreToolUse hook stdout payload, or null when there is nothing to say.
export function formatOutput(nudges) {
  if (!nudges || !nudges.length) return null;
  return JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      additionalContext: nudges.join(" "),
    },
  });
}
```

- [x] **Step 3: Run the test to verify it passes**

Re-run the Step 1 `ctx_execute`. Expected: `ALL PASS`.

- [x] **Step 4: Record** (no git commit — global file). Note in the execution log that Task 1 sandbox tests passed.

---

## Task 2: stdin/stdout glue + fail-open

**Files:**
- Modify: `~/.config/claude/hooks/bash-nudges.mjs` (append the guarded glue)

**Interfaces:**
- Consumes: `analyze`, `formatOutput` from Task 1.
- Produces: an executable hook that reads the PreToolUse stdin JSON (`{tool_name, tool_input:{command}, cwd?}`), and writes the `additionalContext` payload (or nothing), exit 0.

- [x] **Step 1: Write the failing test** (sandbox — exercises the glue's behavior via the pure functions plus a JSON-shape check; the script is not spawned because `node` is denied)

```javascript
const m = await import('/home/mdorman/.config/claude/hooks/bash-nudges.mjs');
// Simulate what the glue does for representative stdin payloads:
function simulate(stdin) {
  try {
    const d = JSON.parse(stdin);
    if (d.tool_name !== 'Bash') return null;
    return m.formatOutput(m.analyze((d.tool_input || {}).command || '', d.cwd || '/home/mdorman/src/jaunder'));
  } catch { return null; }
}
const T = [
  [JSON.stringify({tool_name:'Bash', tool_input:{command:'cd /home/mdorman/src/jaunder && ls'}, cwd:'/home/mdorman/src/jaunder'}), true,  'redundant-cd'],
  [JSON.stringify({tool_name:'Bash', tool_input:{command:'ls'}, cwd:'/home/mdorman/src/jaunder'}),                          false, 'clean'],
  [JSON.stringify({tool_name:'Read', tool_input:{file_path:'/x'}}),                                                          false, 'non-bash'],
  ['{not json',                                                                                                              false, 'malformed'],
];
let fail = 0;
for (const [stdin, wantOut, label] of T) {
  const out = simulate(stdin);
  const ok = (!!out) === wantOut && (!out || /additionalContext/.test(out));
  if (!ok) fail++;
  console.log((ok?'PASS ':'FAIL ')+label);
}
// Confirm the file also defines the main-guard so importing it did NOT exit the process:
console.log(typeof m.analyze === 'function' ? 'PASS import-safe (glue did not run on import)' : 'FAIL import');
console.log(fail===0 ? '\nALL PASS' : '\n'+fail+' FAILURES');
```
Expected now: the `import-safe` assertion FAILS if the glue (added next) runs `process.exit` on import; before the glue exists the simulate cases pass but we have not yet proven the real glue is import-safe. Run it to capture the baseline.

- [x] **Step 2: Append the guarded glue to `bash-nudges.mjs`**

```javascript
import { fileURLToPath } from "node:url";

// Only run the hook glue when executed directly (not when imported by a test).
const invokedAsHook =
  process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (invokedAsHook) {
  let input = "";
  process.stdin.on("data", (c) => (input += c));
  process.stdin.on("end", () => {
    try {
      const data = JSON.parse(input);
      if (data.tool_name === "Bash") {
        const out = formatOutput(
          analyze((data.tool_input || {}).command || "", data.cwd || process.cwd())
        );
        if (out) process.stdout.write(out);
      }
    } catch {
      // fail-open: malformed or unexpected input — stay out of the way.
    }
    process.exit(0);
  });
}
```

- [x] **Step 3: Run the test to verify it passes**

Re-run Step 1. Expected: all simulate cases `PASS` and `PASS import-safe` (the dynamic `import()` returns and the test keeps running, proving the main-guard works). `ALL PASS`.

- [x] **Step 4: Make the file executable** (matches the existing hook's shebang convention)

```bash
chmod +x ~/.config/claude/hooks/bash-nudges.mjs
```
Then prepend the shebang line `#!/usr/bin/env node` as the first line if absent (the file currently starts with `import`). Re-run Step 1 to confirm the shebang did not break import.

---

## Task 3: Activate in global settings (the atomic step) + define live verification

**Files:**
- Modify: `~/.config/claude/settings.json` (add the hook to `PreToolUse`)

**Interfaces:**
- Consumes: the tested `bash-nudges.mjs`.
- Produces: a `PreToolUse` entry (Bash matcher) referencing the hook; serena + context-mode hooks untouched.

- [x] **Step 1: Back up the real settings file**
```bash
cp -p ~/.config/claude/settings.json ~/.config/claude/settings.json.bak
```

- [x] **Step 2: Add the hook to `PreToolUse`** — a new group with a `Bash` matcher, appended after the existing two PreToolUse groups (serena remind, serena auto-approve). Insert before the closing `]` of `PreToolUse`:

```json
      ,{
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "\"/home/mdorman/.config/claude/hooks/bash-nudges.mjs\""
          }
        ]
      }
```

- [x] **Step 3: Validate the settings JSON** (sandbox — `node` CLI is denied)

```javascript
const fs = require('fs');
const j = JSON.parse(fs.readFileSync('/home/mdorman/.config/claude/settings.json','utf8'));
const pre = j.hooks.PreToolUse;
const has = JSON.stringify(pre).includes('bash-nudges.mjs');
const serena = JSON.stringify(pre).includes('serena-hooks');
console.log('valid JSON: OK');
console.log('bash-nudges wired: ' + (has?'OK':'FAIL'));
console.log('serena hooks intact: ' + (serena?'OK':'FAIL'));
console.log('Stop/SessionStart intact: ' + (('Stop' in j.hooks && 'SessionStart' in j.hooks)?'OK':'FAIL'));
```
Expected: all OK. If any FAIL, restore from `.bak` and stop.

- [x] **Step 4: Remove the backup once validated**
```bash
rm ~/.config/claude/settings.json.bak
```

- [x] **Step 5: Document live verification (next session)** — the hook loads only at session start, so it cannot fire in the authoring session. In the next fresh session, confirm:
  1. A Bash call shaped `cd /home/mdorman/src/jaunder && <cmd>` surfaces the G1 nudge and the command still runs (non-blocking).
  2. A Bash call ending `... && echo done` surfaces the G3 nudge and still runs.
  3. `deny-bash-script-runners.mjs` STILL denies a `scripts/...`-style command (i.e. the `additionalContext` hook did not short-circuit the project deny hook). If it did short-circuit, treat as a defect: reorder so the deny hook precedes the nudge hook, or merge the nudge into a single Bash hook chain.
  Record the result; if (3) regresses, the fail-open hook can be removed from `settings.json` instantly (it added no other behavior).

---

## Self-Review

- **Spec coverage:** G1 → `analyze` cd branch (Task 1) + live check (Task 3). G3 → `analyze` echo branch (Task 1) + live check. Fail-open → Task 2 glue + malformed test. Concurrency-safety (build inert, atomic, backup, validate, symlink target) → Global Constraints + Task 3. G2 explicitly deferred (Global Constraints).
- **Placeholders:** none — full hook source, real test code with expected output, exact settings JSON, exact paths.
- **Type consistency:** `analyze(command, cwd) → string[]`, `formatOutput(nudges) → string|null` used identically in Tasks 1, 2, and the glue. The hook reads `tool_name`/`tool_input.command`/`cwd`, matching the verified PreToolUse input contract.
- **Known assumption:** the hook relies on `data.cwd` or `process.cwd()` equalling the project root for G1; if neither holds at runtime, G1 silently no-ops (fail-safe) and Task 3 step 5(1) will reveal it. Documented, not hidden.

## Execution note

P2's only repo artifact is this plan; the hook lives in global config. Mark steps `- [x]` and commit this plan as the execution record, exactly as P1 did.

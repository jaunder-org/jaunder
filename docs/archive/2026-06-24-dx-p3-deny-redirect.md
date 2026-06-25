# DX P3 — Deny-Redirect for Shell-Text Tools (G2 + J4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the bare "security policy" block on `grep`/`head`/`tail`/`sed`/`cat`/`node` with a **deny-that-says-what-to-use-instead**, on BOTH the Bash tool and context-mode's shell tools — killing the ~110× retry dead-loop.

**Architecture:** One new global hook `~/.config/claude/hooks/redirect-tools.mjs`. Pure core `redirectFor(toolName, toolInput) → string|null` extracts the shell text per tool shape (Bash: `command`; `ctx_execute`/`ctx_execute_file`: `code` when `language === "shell"`; `ctx_batch_execute`: each `commands[].command`), matches the offending tool at command position, and returns redirect guidance. Glue denies with that guidance (`permissionDecision: "deny"`), else stays silent. Wired into global `settings.json` under two matcher groups — `Bash` and `mcp__plugin_context-mode_context-mode__ctx_*` — and the five tokens are removed from the project `permissions.deny` (so the hook, not the bare deny, handles them).

**Tech Stack:** Node ESM (`.mjs`); context-mode JS sandbox for tests (the `node` CLI is denied on both shells).

## Global Constraints

- **Decided scope:** both surfaces (Bash + context-mode shell tools). Tools covered: `grep`, `head`, `tail`, `sed`, `cat`, `node`. (`cat` is newly covered; it was not previously in `permissions.deny`.)
- **Why remove from `permissions.deny`:** verified 2026-06-24 that a token in `permissions.deny` produces a bare block that pre-empts any Bash hook, and context-mode mirrors `permissions.deny` (so `grep` is blocked on `ctx_execute` shell too, also bare). A hook can only add guidance for tokens that are NOT in `permissions.deny`. PreToolUse hooks fire before context-mode's internal mirror-check, so the hook restores both-surface coverage *with* guidance.
- **Fail-open:** malformed/unknown input → no output, `exit 0`, tool proceeds. If the hook breaks, these tools simply run unguided (acceptable; not catastrophic).
- **Do NOT deny legit paths:** `rg`, `cargo`, `git`, etc. must pass; `ctx_execute(language:"javascript")` (the sanctioned node path) must pass (only `language === "shell"` is inspected); context-mode non-shell tools (`ctx_search`, `ctx_stats`, …) match the glob but yield no shell text → no-op.
- **Two files, both untracked/local — no repo commit:** global `~/.config/claude/settings.json` (real target of the `~/.claude` symlink) and project `/home/mdorman/src/jaunder/.claude/settings.local.json`. Back up both before editing; validate JSON after. The committed artifact is this plan.
- **Leave the `cd` entries in `permissions.deny` as-is** — they hard-block `cd` to the jaunder root (the G1 nudge from P2 covers other cwds). Out of scope here.
- **Hooks activate immediately** (observed in P2), so live verification happens in-session right after activation.
- No `Co-Authored-By`. Branch `worktree-dx-program`.

### Guidance strings (one per tool)

| Tool match | Guidance |
|------------|----------|
| `grep` | "Use `rg` (respects .gitignore) or the Grep tool instead of `grep`." |
| `head`/`tail` | "Use the Read tool with offset/limit instead of `head`/`tail`." |
| `cat` | "Use the Read tool instead of `cat`." |
| `sed` | "Use Edit for changes / Read for reading ranges instead of `sed`." |
| `node` | "Use ctx_execute(language:\"javascript\") instead of shelling `node`." |

All guidance ends with: " (Denied so you switch tools rather than retry — this is a redirect, not a hard wall.)"

---

## Task 1: Pure `redirectFor` core + unit tests

**Files:**
- Create: `~/.config/claude/hooks/redirect-tools.mjs` (functions only)

**Interfaces:**
- Produces: `export function redirectFor(toolName, toolInput) → string|null` and `export function shellTexts(toolName, toolInput) → string[]`.

- [x] **Step 1: Write the failing test** (sandbox, dynamic import)

`ctx_execute(language: "javascript", code: <below>)`:
```javascript
const m = await import('/home/mdorman/.config/claude/hooks/redirect-tools.mjs');
const CTX = 'mcp__plugin_context-mode_context-mode__ctx_execute';
const BATCH = 'mcp__plugin_context-mode_context-mode__ctx_batch_execute';
const cases = [
  ['Bash', {command:'grep -r foo .'},                       true,  'bash-grep'],
  ['Bash', {command:'cargo build | tail -5'},               true,  'bash-tail-pipe'],
  ['Bash', {command:'sed -i s/a/b/ x'},                     true,  'bash-sed'],
  ['Bash', {command:'cat file.txt'},                        true,  'bash-cat'],
  ['Bash', {command:'node script.js'},                      true,  'bash-node'],
  ['Bash', {command:'rg foo'},                              false, 'bash-rg-ok'],
  ['Bash', {command:'cargo nextest run'},                   false, 'bash-cargo-ok'],
  ['Bash', {command:'git log --grep=foo'},                  false, 'bash-grep-flag-ok'],
  [CTX,  {language:'shell', code:'grep foo bar'},           true,  'ctx-shell-grep'],
  [CTX,  {language:'shell', code:'cargo xtask validate'},   false, 'ctx-shell-cargo-ok'],
  [CTX,  {language:'javascript', code:'require("fs")'},     false, 'ctx-js-ok'],
  [BATCH,{commands:[{label:'a',command:'cargo build'},{label:'b',command:'grep x y'}]}, true, 'batch-grep'],
  [BATCH,{commands:[{label:'a',command:'cargo build'}]},    false, 'batch-clean-ok'],
  ['mcp__plugin_context-mode_context-mode__ctx_search', {queries:['grep']}, false, 'ctx-search-noop'],
  ['Read', {file_path:'/x'},                                false, 'read-noop'],
];
let fail=0;
for (const [t, inp, want, label] of cases) {
  const got = !!m.redirectFor(t, inp);
  const ok = got===want; if(!ok) fail++;
  console.log((ok?'PASS ':'FAIL ')+label+' (want '+want+', got '+got+')');
}
console.log(fail===0?'\nALL PASS':'\n'+fail+' FAILURES');
```
Expected now: import throws → FAIL.

- [x] **Step 2: Write the implementation**

```javascript
// Tool → guidance. Each `re` matches the tool at command position (start, or
// after a pipe / && / ; / newline) so flags like `git log --grep` don't trip it.
const TAIL = " (Denied so you switch tools rather than retry — this is a redirect, not a hard wall.)";
const RULES = [
  { re: /(?:^|[|&;\n]|\&\&)\s*grep\b/, msg: "Use `rg` (respects .gitignore) or the Grep tool instead of `grep`." },
  { re: /(?:^|[|&;\n]|\&\&)\s*(?:head|tail)\b/, msg: "Use the Read tool with offset/limit instead of `head`/`tail`." },
  { re: /(?:^|[|&;\n]|\&\&)\s*cat\b/, msg: "Use the Read tool instead of `cat`." },
  { re: /(?:^|[|&;\n]|\&\&)\s*sed\b/, msg: "Use Edit for changes / Read for reading ranges instead of `sed`." },
  { re: /(?:^|[|&;\n]|\&\&)\s*node\b/, msg: "Use ctx_execute(language:\"javascript\") instead of shelling `node`." },
];

// The shell command strings carried by a tool call (empty for non-shell tools).
export function shellTexts(toolName, toolInput) {
  const ti = toolInput || {};
  if (toolName === "Bash") return [ti.command || ""];
  if (/__ctx_execute$/.test(toolName) || /__ctx_execute_file$/.test(toolName)) {
    return ti.language === "shell" ? [ti.code || ""] : [];
  }
  if (/__ctx_batch_execute$/.test(toolName)) {
    return (Array.isArray(ti.commands) ? ti.commands : []).map((c) => (c && c.command) || "");
  }
  return [];
}

// First matching tool's guidance for any of the call's shell texts, or null.
export function redirectFor(toolName, toolInput) {
  for (const text of shellTexts(toolName, toolInput)) {
    if (typeof text !== "string" || !text.trim()) continue;
    for (const rule of RULES) {
      if (rule.re.test(text)) return rule.msg + TAIL;
    }
  }
  return null;
}
```

- [x] **Step 3: Run the test — expect `ALL PASS`.** If `bash-grep-flag-ok` fails, the `grep` regex is matching `--grep`; confirm the command-position anchor is correct.

- [x] **Step 4: Record** (no commit — global file).

---

## Task 2: Glue + fail-open

**Files:**
- Modify: `~/.config/claude/hooks/redirect-tools.mjs` (shebang + import-guarded glue)

**Interfaces:**
- Consumes: `redirectFor`. Produces: an executable hook emitting deny+guidance or nothing.

- [x] **Step 1: Prepend shebang** `#!/usr/bin/env node` as line 1.

- [x] **Step 2: Append the import-guarded glue**

```javascript
import path from "node:path";
import { fileURLToPath } from "node:url";

const invokedAsHook =
  process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (invokedAsHook) {
  let input = "";
  process.stdin.on("data", (c) => (input += c));
  process.stdin.on("end", () => {
    try {
      const data = JSON.parse(input);
      const guidance = redirectFor(data.tool_name, data.tool_input);
      if (guidance) {
        process.stdout.write(
          JSON.stringify({
            hookSpecificOutput: {
              hookEventName: "PreToolUse",
              permissionDecision: "deny",
              permissionDecisionReason: guidance,
            },
          })
        );
      }
    } catch {
      // fail-open
    }
    process.exit(0);
  });
}
```
(Place the two `import` lines at the top with the file's other imports — none exist yet, so add them directly under the shebang.)

- [x] **Step 3: `chmod +x ~/.config/claude/hooks/redirect-tools.mjs`**

- [x] **Step 4: Real-process + import-safety test** (sandbox)

```javascript
const { spawnSync } = require('child_process');
const HOOK = '/home/mdorman/.config/claude/hooks/redirect-tools.mjs';
const m = await import(HOOK);
console.log(typeof m.redirectFor==='function' ? 'PASS import-safe' : 'FAIL import-safe');
function run(obj){ const r=spawnSync(HOOK,[],{input:JSON.stringify(obj),encoding:'utf8'}); return {code:r.status,out:(r.stdout||'').trim(),error:r.error&&r.error.message}; }
const T = [
  [{tool_name:'Bash',tool_input:{command:'grep -r x .'}}, true, 'bash-grep'],
  [{tool_name:'Bash',tool_input:{command:'rg x'}},        false,'bash-rg'],
  [{tool_name:'mcp__plugin_context-mode_context-mode__ctx_execute',tool_input:{language:'shell',code:'sed -i s/a/b/ f'}}, true, 'ctx-sed'],
  [{tool_name:'mcp__plugin_context-mode_context-mode__ctx_execute',tool_input:{language:'javascript',code:'1+1'}}, false,'ctx-js'],
  ['{bad json', false, 'malformed'],
];
let fail=0;
for (const [obj,want,label] of T) {
  const r = typeof obj==='string' ? (()=>{const rr=spawnSync(HOOK,[],{input:obj,encoding:'utf8'});return{code:rr.status,out:(rr.stdout||'').trim(),error:rr.error&&rr.error.message};})() : run(obj);
  const denied = /"permissionDecision":"deny"/.test(r.out);
  const ok = r.error==null && r.code===0 && denied===want;
  if(!ok) fail++;
  console.log((ok?'PASS ':'FAIL ')+label+' [exit='+r.code+' deny='+denied+']');
}
console.log(fail===0?'\nALL PASS':'\n'+fail+' FAILURES');
```
Expected: `PASS import-safe` and `ALL PASS` (deny on grep/sed shell, allow on rg/js, fail-open exit 0 on malformed).

---

## Task 3: Activate — remove tokens from `permissions.deny`, wire the hook, live-verify

**Files:**
- Modify: `/home/mdorman/src/jaunder/.claude/settings.local.json` (remove 5 deny tokens)
- Modify: `~/.config/claude/settings.json` (add two hook groups)

- [x] **Step 1: Back up both files**
```bash
cp -p ~/.config/claude/settings.json ~/.config/claude/settings.json.bak
cp -p /home/mdorman/src/jaunder/.claude/settings.local.json /home/mdorman/src/jaunder/.claude/settings.local.json.bak
```

- [x] **Step 2: Read then edit the project deny-list.** Read `/home/mdorman/src/jaunder/.claude/settings.local.json`, then remove exactly these five array entries (leave the two `cd` entries and all `allow` entries intact):
`"Bash(sed *)"`, `"Bash(grep *)"`, `"Bash(head *)"`, `"Bash(tail *)"`, `"Bash(node *)"`.

- [x] **Step 3: Add two hook groups to global `~/.config/claude/settings.json` `PreToolUse`.** The `Bash` group already exists (bash-nudges); add redirect-tools to a Bash group AND a context-mode group. Insert before the `PreToolUse` array's closing `]`:
```json
      ,{
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "\"/home/mdorman/.config/claude/hooks/redirect-tools.mjs\"" }
        ]
      },
      {
        "matcher": "mcp__plugin_context-mode_context-mode__ctx_*",
        "hooks": [
          { "type": "command", "command": "\"/home/mdorman/.config/claude/hooks/redirect-tools.mjs\"" }
        ]
      }
```

- [x] **Step 4: Validate both JSON files** (sandbox)
```javascript
const fs=require('fs');
const g=JSON.parse(fs.readFileSync('/home/mdorman/.config/claude/settings.json','utf8'));
const p=JSON.parse(fs.readFileSync('/home/mdorman/src/jaunder/.claude/settings.local.json','utf8'));
const pre=JSON.stringify(g.hooks.PreToolUse);
const deny=p.permissions.deny;
const checks=[
 ['global JSON valid', true],
 ['redirect-tools wired (Bash)', g.hooks.PreToolUse.some(x=>x.matcher==='Bash' && JSON.stringify(x).includes('redirect-tools'))],
 ['redirect-tools wired (ctx)', g.hooks.PreToolUse.some(x=>String(x.matcher).includes('ctx_') && JSON.stringify(x).includes('redirect-tools'))],
 ['bash-nudges still wired', pre.includes('bash-nudges')],
 ['serena hooks intact', pre.includes('serena-hooks remind') && pre.includes('serena-hooks auto-approve')],
 ['deny: grep/head/tail/sed/node removed', !['Bash(grep *)','Bash(head *)','Bash(tail *)','Bash(sed *)','Bash(node *)'].some(t=>deny.includes(t))],
 ['deny: cd entries kept', deny.includes('Bash(cd /home/mdorman/src/jaunder)')],
 ['allow list intact (>0)', (p.permissions.allow||[]).length>0],
];
let ok=true; checks.forEach(([k,v])=>{if(!v)ok=false;console.log((v?'OK ':'FAIL ')+k);});
console.log(ok?'\nALL OK':'\nFAIL — restore from .bak');
```
Expected: ALL OK. If any FAIL, restore both from `.bak` and stop.

- [x] **Step 5: Live-verify in-session** (hooks activate immediately). Confirm:
  1. Bash `grep --version` → now **denied with the rg/Grep guidance** (not the bare "security policy" message).
  2. `ctx_execute(language:"shell", code:"grep --version")` → denied with guidance.
  3. Bash `rg --version` → **runs** (not denied).
  4. `ctx_execute(language:"shell", code:"cargo --version")` → runs.
  5. `ctx_execute(language:"javascript", code:"console.log(1)")` → runs (sanctioned node path untouched).
  Record results. If grep now runs un-denied anywhere (hook didn't catch it), restore from `.bak` and debug before leaving the session.

- [x] **Step 6: Remove both backups once validated + live-verified**
```bash
rm ~/.config/claude/settings.json.bak
rm /home/mdorman/src/jaunder/.claude/settings.local.json.bak
```

---

## Self-Review

- **Spec coverage:** G2 (deny-redirect with guidance) → Tasks 1–3, both surfaces. J4 (project deny-list rework) → Task 3 step 2. Both-surfaces decision → `shellTexts` covers Bash + ctx_execute/_file/_batch; Task 3 wires both matcher groups. Concurrency-safety → backups + validation + fail-open.
- **Placeholders:** none — full source, real tests with expected output, exact tokens to remove, exact JSON to add.
- **Type consistency:** `redirectFor(toolName, toolInput) → string|null` and `shellTexts(toolName, toolInput) → string[]` used identically in core, glue, and tests. Tool-name suffix matches (`__ctx_execute`, `__ctx_execute_file`, `__ctx_batch_execute`) align with the settings matcher glob `ctx_*`.
- **Risk noted:** removing tokens from `permissions.deny` means the hook is the sole guard; fail-open means a hook defect degrades to "tool runs unguided," never to a broken session. Live-verify (step 5) catches a mis-wire before the session ends.

## Execution note

Two untracked config files change; the committed artifact is this plan. Mark steps `- [x]` and commit, as P1/P2 did.

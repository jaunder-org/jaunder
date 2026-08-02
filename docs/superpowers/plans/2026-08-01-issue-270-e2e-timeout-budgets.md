# e2e timeout budget right-sizing — Implementation Plan

> **For agentic workers:** Execute task-by-task with **jaunder-iterate**
> (delegating a task to a subagent via **jaunder-dispatch** when useful). Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`2026-08-01-issue-270-e2e-timeout-budgets.md`](../specs/2026-08-01-issue-270-e2e-timeout-budgets.md)
— criteria referenced as A1–A10. Issue: #270.

**Goal:** Delete the 18 `setTestBudget` sites that protect nothing, and make the
2 that do protect something derive their budgets from the deadlines they cover.

**Architecture:** No mechanism changes. `DEFAULT_TEST_BUDGET_MS`, both browser
scales, `workerContentionScale` and the `_autoTestTimeout` fixture are all
untouched. The work is deletions in six spec files, two derived-budget
expressions in `feeds.spec.ts`, and doc corrections in five files.

**Tech Stack:** TypeScript (`end2end/`), Playwright 1.58.2. Gate is
`cargo xtask check` (runs `tsc --noEmit` + prettier over `end2end/`).

## Review header

**Scope — in:**
`end2end/tests/{feeds,visibility,audiences,atompub,posts,invite}.spec.ts`,
`end2end/tests/{fixtures,helpers}.ts`, `CONTRIBUTING.md`,
`docs/observability.md`, `docs/adr/0012-environment-aware-timeouts.md`,
`flake.nix` (comments only).

**Scope — out:** the 11 `test.slow()` sites; `DEFAULT_TEST_BUDGET_MS`; the
browser scales; `workerContentionScale`'s behaviour; `playwright.config.ts`;
`flake.nix`'s `e2ePlaywrightTimeout`/`e2eGlobalTimeout`.

**Tasks:**

1. File the two follow-ups surfaced this cycle (trace-derived speed
   investigation; budget-drift gate).
2. Delete the 18 `setTestBudget` calls and their now-unused imports.
3. Derive the two surviving budgets from named constants.
4. Correct the docs and comments that teach the removed workflow, plus the stale
   workers=4 commentary and the workers=1 caveat.
5. Verify on all four e2e combos, checking the JSON report rather than trusting
   green.

**Key risks / decisions:**

- **A green gate is not sufficient evidence.** `retries=1` on the warm combos
  reports a whole-test timeout that passes on retry as _flaky, exit 0_. Task 5
  must read `results[].status` and the `flaky-scan` detail, not the exit code.
- **`visibility.spec.ts:112` is the tightest cut** (2.04× measured). If any
  single deletion is going to bite, it is that one.
- **Task 3 raises `feeds:179` from 150 s to 180 s.** Not a mistake: at
  `workers=1` chromium the scale is 1.0, so today's 150 s exactly equals its own
  worst-path poll sum with nothing left for setup. The margin is the point.
- **`workers=1` is accepted risk** (spec, "Accepted risk"). Task 4 records it in
  code so it is not rediscovered.
- **Raising `feeds:179` has a latent interaction with the VM cap.** 180 000 ×
  2.2 = 396 s on firefox, and `JAUNDER_E2E_RETRIES=1` (`flake.nix:947`) could
  double it to 792 s against `e2ePlaywrightTimeout = 1020` (`flake.nix:587`).
  The test measures 11.5 s, so this is latent only — and note Task 5's green
  runs **cannot** surface it, since it only manifests when the test is already
  failing. Noted, not addressed (spec, Out of scope).

---

## Global Constraints

- **Do not change** `DEFAULT_TEST_BUDGET_MS` (30_000), `slowBrowserTimeoutScale`
  (2.2), `slowBrowserFirstNavigationScale` (2.6), or `workerContentionScale`'s
  returns.
- **Preserve verbatim** the `helpers.ts` rule "Do not combine with `test.slow()`
  — the scaled budget already covers Firefox". It is unrelated to this change.
- **Commits:** run `cargo xtask check` first so the pre-commit hook passes clean
  (**jaunder-commit**). Form `type(scope): subject (#270)`. **No
  `Co-Authored-By`.**
- **Gate invocation:**
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets -- <cmd>`,
  then grep the parked log — never `cmd | rg`.
- Expect `cargo xtask check` to reflow Markdown via prettier; stage those edits
  with the task that triggered them.

---

### Task 1: File the follow-ups

**Files:** none (tracker only).

**Interfaces:** produces two issue numbers, referenced in Task 4's comments if
useful.

- [x] **Step 1: File the speed investigation** → filed as **#788**. Note
      discovered while filing: `cargo xtask traces analyze` / `traces run`
      already exist (the #32/#33 ports, both closed), so the issue is "use the
      tooling", not "build it".

Use **jaunder-issues**. Title along the lines of _"e2e: 93% of suite wall-clock
is outside the server — find and cut the client-side cost"_. The evidence is
recorded and reviewed in the **spec's Out-of-scope section** ("Making the suite
faster") — copy it from there rather than from this plan, so the issue carries
numbers that went through spec review:

- Per-combo OTel traces already ship in every CI artifact at
  `.xtask/diagnostics/e2e-<backend>-<browser>/capture-<backend>.tar.gz` →
  `capture/otel-traces.jsonl`, and already link test → app → backend (`e2e.test`
  ×121, `action.timed` ×1233, `request` ×4370, `storage.*`).
- Measured on run 30714621799 (sqlite/chromium): total `e2e.test` **583 980 ms**
  vs total server `request` **42 039 ms** — the server is **7.2%** of
  wall-clock. Median request 1 ms. `crypto.password.hash` 7.8 s total (1.3%),
  `storage.session.authenticate` 15.5 s (2.7%).
- Conclusion to record: backend optimisation is not the lever; the cost is
  client-side (WASM load/hydration per context, render, selector waiting,
  explicit sleeps).
- Note ADR-0028 deferred `analyze-otel-traces` (#32) and
  `run-e2e-trace-analysis` (#33) as host-side xtask work — check whether this
  belongs there before opening a new issue.

Milestone: `Test infrastructure & E2E`. Label `test-infra`.

- [x] **Step 2: File the budget-drift gate** → filed as **#789**.

Title along the lines of _"e2e: gate on test duration approaching its timeout
budget"_. Body: #270 found budgets that had drifted from the deadlines they
cover with nothing to catch it; `flaky.rs` already parses
`playwright-report-<backend>.json`, so the duration and the budget are both in
reach. Design questions to settle in its own cycle: threshold, flake tolerance,
and whether it runs per-combo or on the aggregate. Reference #270. Label
`test-infra`.

- [x] **Step 3: No commit** — tracker-only task.

---

### Task 2: Delete the 18 budgets

**Files (all under `end2end/tests/`):**

- Modify: `feeds.spec.ts` (`:56`, `:119`, `:292`, `:320`), `visibility.spec.ts`
  (`:112`, `:161`, `:217`, `:290`), `audiences.spec.ts` (`:30`, `:218`, `:258`,
  `:281`, `:321`), `atompub.spec.ts` (`:48`, `:82`, `:105`), `posts.spec.ts`
  (`:800`), `invite.spec.ts` (`:35`)

**Interfaces:** consumes nothing; after this task `setTestBudget` has exactly 2
callers, both in `feeds.spec.ts` (Task 3 reworks them).

- [x] **Step 1: Delete the 18 calls**

Remove the `setTestBudget(N);` line at each site listed above, plus the blank
line it leaves if the body now starts with one. **Do not** touch
`feeds.spec.ts:179` or `:245`.

- [x] **Step 2: Drop the now-unused imports**

`visibility.spec.ts`, `audiences.spec.ts`, `atompub.spec.ts`, `posts.spec.ts`
and `invite.spec.ts` no longer call `setTestBudget` at all — remove it from each
file's `from "./fixtures"` import list. `feeds.spec.ts` keeps it (2 callers
remain). `posts.spec.ts` still imports `slowBrowserTimeoutMs` for `:364`/`:636`
— keep that.

- [x] **Step 3: Verify the counts** → 8 lines, exactly as tabulated; no spec
      file other than `feeds.spec.ts`.

```bash
rg -n 'setTestBudget' /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets/end2end/tests/
```

Expected: exactly **8** lines, and no `.spec.ts` file other than
`feeds.spec.ts`:

| Line                             | What it is                                       |
| -------------------------------- | ------------------------------------------------ |
| `fixtures.ts:224`                | the definition — stays                           |
| `fixtures.ts:10`, `:214`, `:324` | three prose mentions — **Task 4** rewrites these |
| `helpers.ts:26`                  | prose mention — **Task 4** rewrites this         |
| `feeds.spec.ts:9`                | the surviving import — stays                     |
| `feeds.spec.ts:179`, `:245`      | the 2 surviving calls — **Task 3** reworks these |

Do **not** treat the four prose mentions or the import as failures here; they
are later tasks' work. A1's real assertion at this step is the negative one: no
`setTestBudget` in `visibility`, `audiences`, `atompub`, `posts` or `invite`.
There is no tooling equivalent — `noUnusedLocals` is off and `setTestBudget`
still exists, so a stale import type-checks clean.

- [x] **Step 4: Gate**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets -- cargo xtask check
```

Expected: PASS. `tsc` is what catches a mistyped or half-removed import.

- [x] **Step 5: Commit** → `571d18a7`, 25 deletions (18 calls + 5 import
      specifiers + 2 stray blank lines).

```bash
git add end2end/tests/feeds.spec.ts end2end/tests/visibility.spec.ts \
        end2end/tests/audiences.spec.ts end2end/tests/atompub.spec.ts \
        end2end/tests/posts.spec.ts end2end/tests/invite.spec.ts \
        docs/superpowers/specs/2026-08-01-issue-270-e2e-timeout-budgets.md \
        docs/superpowers/plans/2026-08-01-issue-270-e2e-timeout-budgets.md
git commit -m "test(e2e): drop 18 whole-test budgets that covered nothing (#270)"
```

Message body should record that 16 of the 18 had no designed-to-be-consumed wait
at all, and the other 2 (`visibility:290` 25 s, `invite:35` 5 s) are covered by
the ambient default with ≥1.5× margin.

---

### Task 3: Derive the two surviving budgets

**Files:** Modify `end2end/tests/feeds.spec.ts` (`:16-31` region, `:179`,
`:245`)

**Line numbers here are pre-change** — as of the fork point, before Task 2. Task
2 deletes `:56` and `:119`, and Step 1 below inserts ~13 lines, so everything
shifts by roughly a dozen. The content anchors (`setTestBudget(90_000);`,
`page.waitForTimeout(2_000)`, `timeoutMs = 25_000`) are unique in the file —
match on those, not on the numbers.

**Interfaces:** produces `FEED_POLL_TIMEOUT_MS`, `FEED_SETUP_ALLOWANCE_MS`,
`PING_WAIT_MS`, `PING_SETTLE_MS` as module constants in `feeds.spec.ts`.

- [ ] **Step 1: Name the poll timeout and derive the `:179` budget**

Replace the literal default on `fetchFeedContaining` with a named constant,
declared next to `FORMATS`:

```ts
/** Per-fetch poll deadline for the eventually-consistent feed cache. The
 *  whole-test budget of the per-user-feeds test is derived from this and
 *  `FORMATS.length`, so adding a format or changing this value carries the budget
 *  with it (#270). */
const FEED_POLL_TIMEOUT_MS = 25_000;

/** Room for what the per-user-feeds test does *besides* polling: two `register()`
 *  cold-WASM navigations, three `createPostViaApi` writes, a logout and a
 *  `waitForURL`. Needed because at `workers=1` the whole-test scale is 1.0, so the
 *  budget is not inflated by the scaler. */
const FEED_SETUP_ALLOWANCE_MS = 30_000;
```

and change the signature at `:27-32` to `timeoutMs = FEED_POLL_TIMEOUT_MS`.

Then at `:179` replace `setTestBudget(150_000);` with:

```ts
// Worst path: one `fetchFeedContaining` per format for each of two users, each
// polling up to FEED_POLL_TIMEOUT_MS. Derived rather than restated so it cannot
// drift from the deadlines it exists to cover.
setTestBudget(
  FORMATS.length * 2 * FEED_POLL_TIMEOUT_MS + FEED_SETUP_ALLOWANCE_MS,
);
```

That evaluates to 180_000 (A2). The increase over today's 150_000 is deliberate
— see the plan header.

- [ ] **Step 2: Name the ping constants and derive the `:245` budget**

Declare next to the others:

```ts
/** How long the WebSub ping test waits for each hub ping to land. */
const PING_WAIT_MS = 40_000;
/** Settle window between the publish wave and the edit wave, always consumed. */
const PING_SETTLE_MS = 2_000;
```

Replace the two `40_000` arguments at `:258-262` and `:283` with `PING_WAIT_MS`,
and `page.waitForTimeout(2_000)` at `:267` with
`page.waitForTimeout(PING_SETTLE_MS)`. Then replace `setTestBudget(90_000);` at
`:245` with:

```ts
// Worst path: two ping waits plus the settle = 82s. The remainder covers
// registration and the two API writes; it is comfortable at workers>=2 (the
// scaler adds 1.5x) and deliberately thin at workers=1.
setTestBudget(2 * PING_WAIT_MS + PING_SETTLE_MS + PING_SETUP_ALLOWANCE_MS);
```

with the allowance named alongside the others, symmetrically with Step 1's:

```ts
/** Room for registration and the two API writes in the ping test. Deliberately
 *  thinner than FEED_SETUP_ALLOWANCE_MS: comfortable once the workers>=2 scaler
 *  applies (135s total vs an 82s worst path), tight at workers=1. */
const PING_SETUP_ALLOWANCE_MS = 8_000;
```

That evaluates to 90_000 — **unchanged behaviour**, now derived (A3).

- [ ] **Step 3: Verify the arithmetic didn't move**

```bash
rg -n -A4 'setTestBudget\(' /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets/end2end/tests/feeds.spec.ts
```

Confirm by inspection: `:179` → 3 × 2 × 25 000 + 30 000 = **180 000**; `:245` →
2 × 40 000 + 2 000 + 8 000 = **90 000**.

- [ ] **Step 4: Gate**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets -- cargo xtask check
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add end2end/tests/feeds.spec.ts
git commit -m "test(e2e): derive the two surviving budgets from their deadlines (#270)"
```

---

### Task 4: Correct the docs, comments and caveats

**Files:**

- Modify: `end2end/tests/helpers.ts:23-27`, `end2end/tests/fixtures.ts` (module
  docstring `:8-11`, `DEFAULT_TEST_BUDGET_MS` docstring `:212-217`,
  `_autoTestTimeout` comment `:320-324`), `CONTRIBUTING.md:285-289`,
  `docs/observability.md:434`, `docs/adr/0012-environment-aware-timeouts.md:30`,
  `flake.nix:792-794` and `:960-963`

- [ ] **Step 1: `fixtures.ts` — the budget docstrings and the workers=1 caveat**

Update **three** places so none instructs the reader to reach for
`setTestBudget` by default — say instead that the ambient budget covers every
test in the suite, and that the two remaining explicit budgets exist because
their tests' internal polling deadlines exceed it:

- `:8-11`, the module docstring;
- `:320-324`, the `_autoTestTimeout` comment;
- **`:214`, inside the `DEFAULT_TEST_BUDGET_MS` docstring** — the sentence
  "Tests needing more call `setTestBudget(ms)`". This is the exact prose A7
  names, and it is easy to miss because the same docstring also receives the A10
  paragraph below.

Add to the `DEFAULT_TEST_BUDGET_MS` docstring (A10):

```
 *  Sized against the suite's measured worst case (#270): at workers=2 this scales to
 *  45s chromium / 66s elsewhere, versus a measured worst of 24.2s / 34.6s. The 18
 *  per-test budgets deleted in #270 were validated at workers>=2. At workers=1 the
 *  contention scale is 1.0, so chromium gets 30s here — and `cargo xtask e2e-local`
 *  defaults to 1 worker against the slower debug wasm build. If the heaviest specs
 *  (visibility, audiences) start timing out there, that is this trade-off surfacing:
 *  run with JAUNDER_E2E_WORKERS=2 or re-add a deliberate budget.
```

- [ ] **Step 2: `helpers.ts:23-27` — rewrite, preserving the `test.slow()`
      rule**

The block currently tells the reader to call `setTestBudget(ms)` for a larger
budget. Rewrite so it describes the ambient budget as sufficient, and a test
needing more as a signal to measure first. **The final clause — "Do not combine
with `test.slow()` — the scaled budget already covers Firefox" — must survive
verbatim** (Global Constraints).

- [ ] **Step 3: The three out-of-tree docs**

`CONTRIBUTING.md:285-289`, `docs/observability.md:434` and
`docs/adr/0012-environment-aware-timeouts.md:30` each teach
`slowBrowserTimeoutMs(testInfo, chromiumBudgetMs)` as the way to set a
_whole-test_ budget — which after this change no spec does. Correct each to
describe the ambient budget, keeping their surrounding content intact. ADR-0012
is an accepted ADR: amend the sentence factually, do not restructure it (same
treatment ADR-0028 got in #229).

- [ ] **Step 4: `flake.nix` — the stale workers=4 commentary**

`:792-794` ("the #155 workers=4 flip sets 4") and `:960-963` ("overriding the
workers=4 gate default") describe a gate default that `:930` already contradicts
("The warm gate runs at workers=2"). Correct both to state what is actually
configured: warm combos run at the config default of 2, cold packages at 1.
**Comments only — no Nix behaviour changes** (A6: `workerContentionScale` keeps
all four rungs).

- [ ] **Step 5: Verify no doc still teaches the removed workflow**

The out-of-tree docs teach **`slowBrowserTimeoutMs`** as the whole-test-budget
idiom, not `setTestBudget` — so probe for that, or the check is vacuous (it
already returns nothing before any work):

```bash
rg -n 'slowBrowserTimeoutMs' /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets/CONTRIBUTING.md /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets/docs/observability.md /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets/docs/adr/0012-environment-aware-timeouts.md
```

Expected: each surviving mention describes it as the
**assertion/first-navigation** scaler (its real remaining use), not as the way
to set a whole-test budget. This is a read, not a count — confirm by inspecting
the three edited regions.

- [ ] **Step 6: Confirm the untouched invariants (A5, A6)**

```bash
git -C /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets diff wt-base-issue-270...HEAD -- end2end/tests/fixtures.ts
```

Expected: the diff touches only comments/docstrings.
`DEFAULT_TEST_BUDGET_MS = 30_000`, `slowBrowserTimeoutScale = 2.2`,
`slowBrowserFirstNavigationScale = 2.6` and all four rungs of
`workerContentionScale` must be **absent from the diff** — that turns A5/A6's
"unchanged" from an assertion into an observation.

- [ ] **Step 7: Gate**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets -- cargo xtask check
```

Expected: PASS. `doc-links` and `adr-format` run inside it, so an ADR edit that
breaks either fails here.

- [ ] **Step 8: Commit** (only after Step 7 is green)

```bash
git add end2end/tests/fixtures.ts end2end/tests/helpers.ts CONTRIBUTING.md \
        docs/observability.md docs/adr/0012-environment-aware-timeouts.md flake.nix
git commit -m "docs(e2e): describe the ambient budget, not the removed per-test one (#270)"
```

`doc-links` and `adr-format` run inside `cargo xtask check`, so an ADR edit that
breaks either fails there.

---

### Task 5: Verify on real e2e runs

**Files:** none — verification only. No commit unless a combo reveals a
regression.

- [ ] **Step 1: Run each combo and check it before starting the next**

`.xtask/last-result.json` is **single-slot** — `xtask/src/result.rs:121`
recreates it on every run — so the flaky count for a combo is destroyed by the
next combo. Run and check one at a time. For each of `sqlite chromium`,
`postgres chromium`, `sqlite firefox`, `postgres firefox`:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets -- cargo xtask e2e <backend> <browser>
```

then immediately run Steps 2 and 3 for that combo before moving on.

**One at a time, not `validate`.** Four concurrent VMs on one box distort
durations and have already blown the Playwright cap once this session on an
unmodified tree. Use the Bash tool's background mode; each takes ~10 min.

- [ ] **Step 2: Check that combo's report for timeouts — the actual A9 check**

```bash
jq '[.. | objects | select(has("status") and has("duration")) | .status] | group_by(.) | map({(.[0]): length}) | add' \
  /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets/.xtask/diagnostics/e2e-<backend>-<browser>/playwright-report-<backend>.json
```

Expected: `{"passed": N}` with **no `"timedOut"` key**. This filter selects
`results[]` entries specifically — they are the only objects carrying both
`status` and `duration`. Do not substitute `tests[].status`: that level is
`expected|unexpected|flaky|skipped` and is **never** `timedOut`, so it passes
vacuously. A missing report file means the combo died before writing one — treat
as a failure, not a pass.

- [ ] **Step 3: Check that combo's flaky count, before running the next combo**

```bash
jq -r '.steps[] | select(.name=="flaky-scan") | .detail' \
  /home/mdorman/src/jaunder/.claude/worktrees/issue-270-e2e-timeout-budgets/.xtask/last-result.json
```

Expected: `0 flaky test(s)`. The step is **always** `ok` (`flaky.rs:42`), so its
status proves nothing — the count lives only in `detail`. This is the
load-bearing check: `retries=1` on the warm combos turns a whole-test timeout
that passes on retry into a green run, which is exactly the regression this
cycle risks.

- [ ] **Step 4: If a combo fails or reports a timeout**

The likely culprit is `visibility.spec.ts:112` (2.04× measured margin). Do
**not** reach for raising `DEFAULT_TEST_BUDGET_MS` — that inflates the 11
`test.slow()` sites 3:1. Re-add a deliberate `setTestBudget` to the specific
test, sized from its intentional waits plus setup, and record it in the spec's
table.

---

## Self-review

**Spec coverage.** A1 → Task 2 Steps 1–3. A2 → Task 3 Step 1 (additive named
allowance; the spec's A2 was amended to permit that over a multiplicative
factor, and says why). A3 → Task 3 Step 2. A4 → the spec's committed table
(evidence, not an implementation step) plus Task 2's deletions. A5 and A6 → Task
4 Step 6's diff read, backed by Global Constraints; A6's comment half → Task 4
Step 4. A7 → Task 4 Steps 1–3 and Step 5's `slowBrowserTimeoutMs` probe. A8 →
the gate step in Tasks 2, 3 and Task 4 Step 7. A9 → Task 5 Steps 2–3, run per
combo. A10 → Task 4 Step 1. Every criterion maps, and the three "unchanged"
criteria are now observed rather than asserted.

**Type consistency.** `FEED_POLL_TIMEOUT_MS`, `FEED_SETUP_ALLOWANCE_MS`,
`PING_WAIT_MS` and `PING_SETTLE_MS` are declared in Task 3 Steps 1–2 and used
only there and at their call sites in the same file. No task references a symbol
another task removes: Task 2 leaves `setTestBudget` imported in `feeds.spec.ts`
precisely because Task 3 still calls it.

**No placeholders.** Every step names exact files, exact line regions, the
literal text to write, and the command with its expected output. Task 5 Step 4
gives the named fallback rather than "handle failures".

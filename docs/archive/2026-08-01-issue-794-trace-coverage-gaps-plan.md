# E2e trace-coverage gaps (#794) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the 28–31 % of per-test e2e wall-clock that is currently
invisible attributable in the trace, and close the seven other trace-coverage
gaps the #788 investigation found.

**Architecture:** Three seams, in dependency order. (1) One instrumented polling
primitive replaces six copied loops. (2) One context-level `attachTraceCapture`
with a phase-tagged sink replaces the instrumentation inlined in
`_autoPerfSpan`, which is what lets a lifecycle envelope wrap `e2e.test` without
moving any of its values. (3) `performance.mark`s emitted from the CSR boot
path, harvested per navigation and read back by prefix, plus two derived
metrics, decompose `commit_to_mount`.

**Tech Stack:** TypeScript (Playwright 1.58.2) under `end2end/tests/`; Rust
(`client`, `csr`, `xtask`).

**Spec:** `docs/superpowers/specs/2026-08-01-issue-794-trace-coverage-gaps.md` —
referenced by decision (D1…D8) and criterion (AC-1…AC-23). Don't re-derive
rationale.

**ADR draft:** `docs/adr/0096-e2e-trace-capture-vs-attribution.md`.

## Global Constraints

- **Nothing may change `e2e.test`'s span id, time range, attribute keys, or the
  values of `e2e.request_count` / `e2e.navigation_count` / `e2e.action_count`**
  (AC-3).
- **`docs/coverage/server-fns.json` must regenerate byte-identical**; orphan
  reason set unchanged (AC-14).
- **Every new `e2e.*`-named span MUST carry an `e2e.project` attribute.**
  `parse.rs:134` drops any `e2e.`-prefixed span whose `e2e.project` differs from
  a `--project` filter — an unstamped span reads as "wrong project" and the
  whole lifecycle tree vanishes under `traces analyze --project firefox`.
- **No `Co-Authored-By` trailer.** Marks unconditional (D6). Poll timeouts
  per-call (D4).
- Commands run via
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-794-trace-coverage-gaps -- <cmd>`.
- Before each commit run `cargo xtask check` (**jaunder-commit**). Stage, then
  commit.

### Verification constraint — `e2e-local` exports NO spans

`exportSpans` is a **silent no-op** unless `JAUNDER_E2E_OTLP_HTTP_ENDPOINT` or
`JAUNDER_E2E_TRACEPARENT` is set (`end2end/tests/otel.ts:180-183`).
`cargo xtask e2e-local` sets neither (`xtask/src/steps/e2e_local.rs:193-198`);
only the VM path does (`flake.nix:616-618`).

- **`cargo xtask e2e-local <spec>`** → pass/fail of a spec. Never for inspecting
  spans.
- **`cargo xtask e2e sqlite chromium`** → the only local command producing a
  capture. Writes
  `.xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz`, the fixed path
  `server-fn-coverage` reads (`xtask/src/server_fn_coverage/io.rs:28`) — so it
  must be **re-run** before each `verify`, or `verify` silently checks a stale
  capture.

### `-p xtask` does not work

`xtask/Cargo.toml` declares its own `[workspace]`, so xtask is **not** a member
of the root workspace: `cargo … -p xtask` fails with "package ID specification
`xtask` did not match any packages" (and unhelpfully suggests `maik`). Use
`--manifest-path xtask/Cargo.toml`. `-p client` is fine — `client` IS a root
workspace member.

## Review header

**Scope — in:** all eight gaps, in `end2end/tests/`, `client/`, `csr/`,
`xtask/src/traces/`, `docs/observability.md`. **Scope — out:** acting on the
instrumentation (#791/#792/#793); cutting mount cost (#801); a gate on residual
invisible time.

**Tasks:** 1 file separable concerns · 2 nullable-page `withTimedAction` · 3
polling primitive · 4 composite flows · 5 `attachTraceCapture` + phase sink · 6
lifecycle envelope · 7 `e2e.page` spans · 8 truncation counts · 9 CSR boot marks
· 10 derived mount phases · 11 `traces analyze` coverage section · 12 docs · 13
measurement.

**Key risks:**

- **Task 5 can silently break AC-3.** Requests are recorded on `requestfinished`
  (`fixtures.ts:552`), so a naive sink swap puts warmup requests still in flight
  into the test sink. The fix is structural: tag each request with the active
  phase at the **`request`** event (`:527`), which is where navigations are
  already bucketed (`:538`). Run-to-run count variance under `workers=2` +
  retries means a one-shot capture diff cannot reliably detect a small leak —
  the structural fix is the guarantee, the diff is confirmation.
- **Task 5 must seed over `context.pages()`.** `context.on("page")` does not
  replay for already-created pages, and `_autoPerfSpan` declares `{ page }` so
  the default page exists before capture attaches. Missing this empties
  `navigations[]`.
- **Task 9** is the only task touching shipped app code.

---

### Task 1: File the separable concerns — **DONE**

- [x] **Step 1:** Filed #801 (mount-cost, #788 lever 4), typed `Task`, labelled
      `test-infra`, milestone "Test infrastructure & E2E", added to Backlog #1.
- [x] **Step 2:** Body records both corrections to #788's premises — that
      `commit_to_mount` excludes the mount-path fetches
      (`csr/src/lib.rs:49-54`), and that those fetches are per-route and partly
      serialized (`web/src/cockpit/component.rs:36` awaits `session.reconcile`).
- [ ] **Step 3:** Comment on #788 pointing at #801 as lever 4's home.

---

### Task 2: `withTimedAction` accepts a nullable page

**Files:** Modify `end2end/tests/actions.ts:30-68` **Produces:**
`withTimedAction<T>(page: Page | null, name: string, action: () => Promise<T>): Promise<T>`

- [ ] **Step 1:** `page: Page | null`; the two `pageUrl: page.url()` sites
      (`:48`, `:62`) become `page?.url()`. `ActionRecord.pageUrl` is already
      optional (`:19`). Widening — no caller changes.
- [ ] **Step 2:** Run `cargo xtask check --no-test` → PASS
- [ ] **Step 3:** Commit

```bash
git add end2end/tests/actions.ts
git commit -m "refactor(e2e): withTimedAction accepts a page-less action (#794)"
```

---

### Task 3: `pollUntil`, all six poll sites, the settle sleep

Closes gap 6 (AC-16 … AC-20).

**Files:** Create `end2end/tests/polling.ts`, `end2end/tests/polling.spec.ts`,
`end2end/tests/feeds.ts`. Modify `mail.ts:59-74`, `websub.ts:57-99`,
`fixtures.ts:373-397`, `visibility.spec.ts:310-325`, `feeds.spec.ts:27-49`,
`:267`.

**Produces:**

```ts
export async function pollUntil<T>(
  name: string,
  probe: () => T | undefined | Promise<T | undefined>,
  opts: { intervalMs: number; timeoutMs: number; describe: string },
): Promise<T>;

/** Non-throwing sibling: resolves `undefined` on timeout so the caller's own
 *  assertion produces the failure diff. */
export async function pollUntilOrUndefined<T>(
  name: string,
  probe: () => T | undefined | Promise<T | undefined>,
  opts: { intervalMs: number; timeoutMs: number },
): Promise<T | undefined>;
```

- [ ] **Step 1: Write the failing tests** in `polling.spec.ts` (browser-free —
      declares no `page` fixture; picked up by the default `chromium` project,
      whose `testIgnore` only excludes admin/invite).

```ts
import { test, expect } from "./fixtures";
import { pollUntil, pollUntilOrUndefined } from "./polling";

test("pollUntil returns the first non-undefined probe value", async () => {
  let calls = 0;
  const got = await pollUntil(
    "wait.test",
    () => (++calls < 3 ? undefined : "ok"),
    {
      intervalMs: 10,
      timeoutMs: 2_000,
      describe: "a value",
    },
  );
  expect(got).toBe("ok");
  expect(calls).toBe(3);
});

test("pollUntil rethrows a first-probe failure immediately (AC-19)", async () => {
  const startedMs = Date.now();
  await expect(
    pollUntil(
      "wait.test",
      () => {
        throw new Error("capture-path unset");
      },
      {
        intervalMs: 250,
        timeoutMs: 30_000,
        describe: "a value",
      },
    ),
  ).rejects.toThrow("capture-path unset");
  // The point of AC-19: a misconfigured run must NOT burn the full timeout.
  expect(Date.now() - startedMs).toBeLessThan(1_000);
});

test("pollUntil throws on timeout", async () => {
  await expect(
    pollUntil("wait.test", () => undefined, {
      intervalMs: 10,
      timeoutMs: 200,
      describe: "never arrives",
    }),
  ).rejects.toThrow();
});

test("pollUntilOrUndefined resolves undefined on timeout", async () => {
  const got = await pollUntilOrUndefined("wait.test", () => undefined, {
    intervalMs: 10,
    timeoutMs: 200,
  });
  expect(got).toBeUndefined();
});
```

- [ ] **Step 2:** Run `cargo xtask e2e-local polling.spec.ts` → FAIL
      (`./polling` absent)

- [ ] **Step 3: Implement `polling.ts`**

```ts
import { expect } from "@playwright/test";
import { withTimedAction } from "./actions";

export async function pollUntil<T>(
  name: string,
  probe: () => T | undefined | Promise<T | undefined>,
  opts: { intervalMs: number; timeoutMs: number; describe: string },
): Promise<T> {
  return withTimedAction(null, name, async () => {
    // The FIRST probe runs OUTSIDE toPass, which retries on *any* throw. Without
    // this, a misconfigured run (capturePathViaTool throws when
    // JAUNDER_CAPTURE_DIR is unset — capture.ts:7-10) degrades from an instant
    // stack trace to a full-timeout failure. AC-19.
    const first = await probe();
    if (first !== undefined) return first;

    let found: T | undefined;
    await expect(async () => {
      found = await probe();
      expect(found, opts.describe).not.toBeUndefined();
    }).toPass({ timeout: opts.timeoutMs, intervals: [opts.intervalMs] });
    return found as T;
  });
}
```

`pollUntilOrUndefined` wraps `pollUntil` and returns `undefined` on rejection.

- [ ] **Step 4:** Run `cargo xtask e2e-local polling.spec.ts` → PASS

- [ ] **Step 5: Rewrite the four capture-file sites.** Signatures, defaults and
      intervals unchanged; only the loop body moves. `mail.ts:59` →
      `"wait.mail"` (100 ms); `fixtures.ts:381` likewise (cursor stays in the
      probe); `websub.ts:57` and `:82` → `"wait.websub_ping"` (250 ms).
      **Resolve `mailCaptureFile()` / `websubCaptureFile()` before the
      `pollUntil` call**, so the memoized `capturePathViaTool` throw is not
      inside the probe at all.

- [ ] **Step 6: Promote the feed poll to `feeds.ts`** (AC-11c).
      `feeds.spec.ts:27` `fetchFeedContaining` and `visibility.spec.ts:313` are
      the same sequence. `fetchFeedContaining` uses `pollUntil`;
      **`visibility.spec.ts` uses `pollUntilOrUndefined`** and keeps both
      existing assertions at the call site — today the loop `break`s and falls
      through, so on timeout
      `expect(body, "feed contains the Public post").toContain(...)` produces a
      diff **and** `not.toContain("Feed Subscribers Only")` (`:326`) still runs.
      A throwing poll would lose both. AC-20 is about that behaviour, not just
      the string.

- [ ] **Step 7: Wrap the settle sleep** (AC-18). `feeds.spec.ts:267` →
      `withTimedAction(page, "wait.settle", () => page.waitForTimeout(2_000))`,
      with a comment: #794 makes it visible, #793 removes it.

- [ ] **Step 8: Verify the sweep.** Run
      `rg -n 'setTimeout\(resolve|waitForTimeout' end2end/tests/` Expected:
      exactly **one** match — the wrapped `waitForTimeout` in `feeds.spec.ts`.
      Zero `setTimeout(resolve` anywhere, including `polling.ts` (`toPass` owns
      the interval).

- [ ] **Step 9:** Run `cargo xtask e2e-local feeds.spec.ts`, then
      `visibility.spec.ts`, then `password_reset.spec.ts` → all PASS

- [ ] **Step 10: Commit**

```bash
git add end2end/tests/polling.ts end2end/tests/polling.spec.ts end2end/tests/feeds.ts end2end/tests/mail.ts end2end/tests/websub.ts end2end/tests/fixtures.ts end2end/tests/visibility.spec.ts end2end/tests/feeds.spec.ts
git commit -m "test(e2e): one instrumented polling primitive replaces six copied loops (#794)"
```

---

### Task 4: Composite flows

Closes gap 3 (AC-10, AC-11a/b/d; AC-11c landed in Task 3).

**Files:** Modify `helpers.ts:151-300`, `posts.ts:55`, `fixtures.ts:402-421`,
`email.spec.ts`, `password_reset.spec.ts`

**Produces:** `setAndVerifyEmail(page, email, mailbox)`,
`requestPasswordReset(page, username)`,
`registerAndLogin(page, firstNavTimeoutMs)`

- [ ] **Step 1: Wrap six existing helpers** (AC-10) — same shape `register` uses
      at `helpers.ts:197`; bodies unchanged. `flow.login` (`:169`),
      `flow.subscribe` (`:243`), `flow.unsubscribe` (`:256`),
      `flow.follow_email_link` (`:289`), `flow.fill_login_form` (`:151`),
      `flow.compose_post` (`posts.ts:55`).
- [ ] **Step 2: `setAndVerifyEmail`** (AC-11a) — body is `fixtures.ts:411-418`
      verbatim, wrapped `flow.verify_email`. Called by `verifiedUser` and by
      `email.spec.ts`, which keeps its own assertions.
- [ ] **Step 3: `requestPasswordReset`** (AC-11b) — body is goto
      `/forgot-password`, fill username, submit; wrapped
      `flow.request_password_reset`. **Two** inline copies exist, not one:
      `password_reset.spec.ts:19-21` **and `:84-87`** (the "user without
      verified email" test). Both call the helper; each keeps its own assertion
      (`:24` neutral confirmation; `:88-89` error visible).
- [ ] **Step 4: `registerAndLogin`** (AC-11d) — `registerKnown` then `login`,
      wrapped `flow.register_and_login`.
- [ ] **Step 5: Verify zero inline copies** (AC-11) — one `rg` per sequence:
      `rg -n 'profile/email' end2end/tests/ --glob '*.spec.ts'`;
      `rg -n 'forgot-password' end2end/tests/ --glob '*.spec.ts'`;
      `rg -n 'registerKnown' end2end/tests/ --glob '*.spec.ts'`;
      `rg -n 'feed\.atom|feed\.rss' end2end/tests/ --glob '*.spec.ts'`.
      Expected: each matches only helper call sites and assertions — no
      remaining fill+submit+wait sequence.
- [ ] **Step 6:** Run `cargo xtask e2e-local email.spec.ts`,
      `password_reset.spec.ts`, `auth.spec.ts` → all PASS
- [ ] **Step 7: Commit**

```bash
git add end2end/tests/helpers.ts end2end/tests/posts.ts end2end/tests/fixtures.ts end2end/tests/email.spec.ts end2end/tests/password_reset.spec.ts
git commit -m "test(e2e): delimit composite flows and factor repeated sequences (#794)"
```

---

### Task 5: `attachTraceCapture` with a phase-tagged sink

Closes AC-12; precondition for 6, 7, 10. Implements D1 and D1a.

**Files:** Create `end2end/tests/capture-trace.ts`. Modify
`fixtures.ts:311-318`, `:423-530`.

**Produces:**

```ts
export type Phase = "warmup" | "test";
export type CaptureSink = {
  requests: RequestRecord[];
  navigations: NavigationRecord[];
};
export type TraceCapture = {
  /** Records STARTED after this call are routed to `phase`'s sink. */
  setPhase(phase: Phase): void;
  sinkFor(phase: Phase): CaptureSink;
  readPagePerf(page: Page): Promise<PagePerfSummary>;
  /** Marks harvested per navigation (Task 10). */
  marksFor(navigationId: number): Array<{ name: string; startTime: number }>;
};
export async function attachTraceCapture(
  context: BrowserContext,
): Promise<TraceCapture>;
```

`RequestRecord`, `NavigationRecord`, `PagePerfSummary` move here from
`fixtures.ts:39-87` and are re-exported.

- [ ] **Step 1: Move the instrumentation, retargeted to the context.**
      `context.exposeBinding("__jaunderRecordMount", …)`,
      `context.addInitScript(…, MOUNTED_ATTR)`,
      `context.on("request"|"requestfinished"|"requestfailed", …)`.

- [ ] **Step 2: Attach page-scoped listeners to existing AND future pages.**
      `framenavigated` / `domcontentloaded` / `load` have no context-level
      equivalent, and `context.on("page")` does **not** replay for pages that
      already exist — `_autoPerfSpan` declares `{ page }` (`fixtures.ts:424`),
      so the default page is already created when capture attaches. Both paths
      are required:

```ts
const attachPage = (p: Page) => {
  /* framenavigated / domcontentloaded / load */
};
context.pages().forEach(attachPage); // the default page — MUST NOT be omitted
context.on("page", attachPage); // pages a spec opens later
```

      Omitting the first line leaves `navigations[]` empty and drives
      `e2e.navigation_count` to 0, breaking AC-3.

- [ ] **Step 3: Tag each request with its phase at the `request` event, not on
      completion.** Requests are pushed on `requestfinished`
      (`fixtures.ts:552`), so routing "records" by current phase puts warmup
      requests **still in flight at the swap** into the test sink — warmup kicks
      off wasm/JS fetches it does not await. Navigations already bucket
      correctly because they are pushed on `request` (`:538`). Capture the phase
      into `requestStarts` at `request` time and route on completion by that
      stored tag:

```ts
const requestPhase = new Map<Request, Phase>();
context.on("request", (r) => {
  requestStarts.set(r, Date.now());
  requestPhase.set(r, phase);
});
context.on("requestfinished", (r) => {
  sinks[requestPhase.get(r) ?? phase].requests.push(rec);
});
```

- [ ] **Step 4: Call from `_autoPerfSpan`, switching phase with the
      traceparent**

```ts
const capture = await attachTraceCapture(page.context());
await warmupPageContext(page, testInfo); // phase "warmup"
capture.setPhase("test"); // ← same moment as the traceparent
await applyTestTraceparent(page.context(), traceContext.traceId, testSpanId);
const testStartMs = Date.now();
```

      The teardown reads `capture.sinkFor("test")` wherever it read the old locals,
      so `e2e.test`'s attributes derive from exactly the same records as before.

- [ ] **Step 5: Call from `tracedContext`**, wrapping `close()` so the
      client-side perf snapshot can be taken while the page is still alive (Task
      7 needs this; `context.on("close")` fires _after_ close, when
      `page.evaluate` throws):

```ts
tracedContext: async ({ browser, testSpanId }, use) => {
  const { traceId } = traceContextFromEnvironment();
  await use(async (options) => {
    const context = await browser.newContext(options);
    await applyTestTraceparent(context, traceId, testSpanId);
    const capture = await attachTraceCapture(context);
    capture.setPhase("test");
    const close = context.close.bind(context);
    context.close = async (...a) => { await snapshot(context, capture); return close(...a); };
    extraContexts.push({ context, capture });
    return context;
  });
},
```

- [ ] **Step 6: Produce the AC-3 baseline, then compare.** The baseline is not
      optional — without it Step 7 has nothing to diff against.

```bash
git stash push -u -m issue-794-ac3-baseline
```

      Run `cargo xtask e2e sqlite chromium`, copy
      `.xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz` to
      `/tmp/claude-1000/-home-mdorman-src-jaunder/4948be11-719d-4cfe-8725-80a76090dea9/scratchpad/baseline-capture.tar.gz`,
      then restore by SHA per the stash rules (`git stash list --format='%H %gs'`,
      `git stash apply <sha>`, then drop by tag).

- [ ] **Step 7: Compare counts** (AC-3). Run `cargo xtask e2e sqlite chromium`
      on the branch; for every test title compare `e2e.request_count`,
      `e2e.navigation_count`, `e2e.action_count` against the baseline. Expected:
      **no systematic shift.** Counts vary run-to-run (`workers=2`,
      `JAUNDER_E2E_RETRIES=1` — `flake.nix:947`), so a ±1 request wobble is
      noise; a consistent **+1 navigation and ~+10 requests per test** is warmup
      leaking, i.e. Step 3 or Step 4 is wrong. The structural fix in Step 3 is
      the guarantee; this diff is confirmation, not the guard.

- [ ] **Step 8: Verify the coverage gate** (AC-14) —
      `cargo xtask server-fn-coverage     verify` against the capture just
      produced in Step 7 → PASS

- [ ] **Step 9: Commit**

```bash
git add end2end/tests/capture-trace.ts end2end/tests/fixtures.ts
git commit -m "test(e2e): one context-level trace capture with a phase-tagged sink (#794)"
```

---

### Task 6: Lifecycle envelope and phase spans

Closes gap 1 (AC-1, AC-2). Implements D2, D2a.

**Files:** Modify `fixtures.ts:325-333`, `:423-893`

- [ ] **Step 1: Stamp the lifecycle start** (D2a). Add a `lifecycleStartMs`
      fixture populated by `_autoTestTimeout` (`:325`) — the auto fixture with
      no `page` dependency, so its stamp precedes context/page creation. **Add a
      comment recording why the ordering holds and how it breaks:** Playwright
      sets up auto fixtures in _registration_ order (a Map, stable-sorted
      worker-before-test), so the guarantee rests on this key preceding
      `_autoPerfSpan` in the same `base.extend({…})` literal. Reordering the
      keys silently collapses `e2e.context_mint` to zero width — which Step 3's
      non-zero assertion catches.

- [ ] **Step 2: Build four spans at teardown**, exported in the same
      `exportSpans` call. `e2e.test.lifecycle` gets a fresh id and parents the
      other three **and** `e2e.test`; `e2e.test` keeps its own id (`testSpanId`,
      `:877`) and range, gaining only a `parentSpanId`. Safe per D2
      (`analyze.rs:66,450,472` match the name exactly; `extract.rs:47-53,76`
      walks upward).

| Span                 | Start                      | End                             |
| -------------------- | -------------------------- | ------------------------------- |
| `e2e.test.lifecycle` | `lifecycleStartMs`         | just before `exportSpans`       |
| `e2e.context_mint`   | `lifecycleStartMs`         | `_autoPerfSpan` entry           |
| `e2e.warmup`         | before `warmupPageContext` | after it (omit when warmup off) |
| `e2e.teardown`       | `endMs` (`:647`)           | just before `exportSpans`       |

      **Every one carries `e2e.project`** (Global Constraints) — plus `e2e.file` and
      `e2e.test` for joining. `e2e.warmup` carries the warmup sink's request and
      navigation counts, so warmup is measured while staying unattributed for #681.

- [ ] **Step 3: Verify shape** (AC-1, AC-2). Run
      `cargo xtask e2e sqlite chromium` (**not** `e2e-local` — it exports no
      spans). In the capture: every test has exactly one `e2e.test.lifecycle`,
      one `e2e.context_mint`, one `e2e.teardown`; `e2e.warmup` present (warmup
      is on in the gate); `e2e.context_mint` **duration non-zero**, starting at
      the lifecycle start.

- [ ] **Step 4: Verify `--project` still sees the tree** (Global Constraints).
      Run
      `cargo xtask traces analyze <the capture's otel-traces.jsonl> --project chromium`
      Expected: the lifecycle spans appear. If they vanish, `e2e.project` is
      unstamped.

- [ ] **Step 5: Re-verify AC-3 and AC-14 after reparenting.** Step 3 already
      produced a fresh capture, so `server-fn-coverage verify` now reads _this_
      task's output rather than Task 5's. Run it → PASS. Then repeat Task 5 Step
      7's count comparison against the same baseline — reparenting must not move
      counts either.

- [ ] **Step 6: Commit**

```bash
git add end2end/tests/fixtures.ts
git commit -m "test(e2e): lifecycle envelope span makes fixture phases attributable (#794)"
```

---

### Task 7: `e2e.page` spans

Closes AC-13.

**Files:** Modify `fixtures.ts`, `capture-trace.ts`

- [ ] **Step 1:** Build one `e2e.page` span per registered extra context (the
      `extraContexts` list from Task 5 Step 5), parented to
      `e2e.test.lifecycle`, carrying that context's `navigation_count`,
      `request_count`, `navigation_top_json`, `resource_summary_json` — **and
      `e2e.project`**. The perf snapshot comes from the wrapped `close()`, since
      `on("close")` is too late.
- [ ] **Step 2: Measure N** (AC-13). Run `cargo xtask e2e sqlite chromium`; sum
      `navigation_count` over the Private-post visibility test's `e2e.test` +
      all its `e2e.page` spans. **Write that literal into the spec at AC-13**,
      replacing the placeholder. Deliberately not an equality against
      `page.goto` count — `pushState` routing produces navigations with no
      `goto`, and an aborted or same-document `goto` may not commit.
- [ ] **Step 3: Commit**

```bash
git add end2end/tests/fixtures.ts end2end/tests/capture-trace.ts docs/superpowers/specs/2026-08-01-issue-794-trace-coverage-gaps.md
git commit -m "test(e2e): per-page spans so multi-context tests stop under-reporting (#794)"
```

---

### Task 8: Truncation dropped-counts

Closes AC-15. D7's table: five capped lists, two of them silent.

**Files:** Modify `fixtures.ts:673-682`, `:711-760`, `:762-799`

- [ ] **Step 1:** `e2e.request_top_slow_dropped`, `e2e.action_top_dropped`,
      `e2e.navigation_top_dropped` = full length minus sliced length.
- [ ] **Step 2:** Inside the `page.evaluate`, return a dropped count alongside
      `resources.topSlow` (`:682`, slice 20) and `__jaunderLongTasks` (`:673`,
      **`slice(-20)`** — a tail slice, so the _earliest_ long tasks vanish with
      no other attribute recording it). Emit `e2e.resource_top_slow_dropped`,
      `e2e.long_tasks_dropped`.
- [ ] **Step 3: Verify** — run `cargo xtask e2e sqlite chromium`; at least one
      heavy timeline test in `posts.spec.ts` reports
      `e2e.action_top_dropped > 0`, and tests under the caps report `0`, not
      absent.
- [ ] **Step 4: Commit**

```bash
git add end2end/tests/fixtures.ts
git commit -m "test(e2e): record dropped entries so trace truncation is never silent (#794)"
```

---

### Task 9: CSR boot marks via `client::perf`

Closes AC-5, AC-9. Only task touching shipped app code.

**Files:** Create `client/src/perf.rs`. Modify `client/Cargo.toml:9-24`,
`client/src/lib.rs`, `csr/src/lib.rs:31-54`

**Produces:**
`client::perf::{mark, MARK_PREFIX, BOOT_ENTRY, BOOT_SEED_PARSED, BOOT_RENDER_START, BOOT_MOUNT_DONE}`.
`csr` reaches it via its existing path dep (`csr/Cargo.toml:12`) — no direct
web-sys dependency needed.

- [ ] **Step 1: Write the failing tests** (host-testable half):

```rust
pub const MARK_PREFIX: &str = "jaunder.";
pub const BOOT_ENTRY: &str = "jaunder.boot.entry";
pub const BOOT_SEED_PARSED: &str = "jaunder.boot.seed_parsed";
pub const BOOT_RENDER_START: &str = "jaunder.boot.render_start";
pub const BOOT_MOUNT_DONE: &str = "jaunder.boot.mount_done";

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&str; 4] = [BOOT_ENTRY, BOOT_SEED_PARSED, BOOT_RENDER_START, BOOT_MOUNT_DONE];

    #[test]
    fn every_boot_mark_carries_the_discovery_prefix() {
        for name in ALL {
            assert!(name.starts_with(MARK_PREFIX), "{name} is invisible to prefix discovery");
        }
    }

    #[test]
    fn boot_mark_names_are_distinct() {
        let mut sorted = ALL.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ALL.len(), "duplicate mark name in {ALL:?}");
    }
}
```

- [ ] **Step 2:** Run `cargo nextest run -p client perf` → FAIL (module absent)
- [ ] **Step 3:** Add `"Performance"` to `client/Cargo.toml`'s web-sys features.
      Implement `pub fn mark(name: &str)`, `#[cfg(target_arch = "wasm32")]` with
      a host no-op counterpart so the tests compile. Both `window()` and
      `performance()` degrade to a no-op — never unwrap; a missing Performance
      API must not break boot.
- [ ] **Step 4:** Run `cargo nextest run -p client perf` → PASS
- [ ] **Step 5:** Emit from `csr/src/lib.rs`: `BOOT_ENTRY` first statement of
      `main()`; `BOOT_SEED_PARSED` after the seed `and_then` (`:33`);
      `BOOT_RENDER_START` immediately before `mount_to_body` (`:42`);
      `BOOT_MOUNT_DONE` after `mount()` returns, before `mark_ready()` (`:53`).
      Unconditional (D6).
- [ ] **Step 6: Verify in the RELEASE bundle** (AC-9). `cargo xtask build-csr`
      defaults to **debug** (`build_csr.rs:27-31`, `lib.rs:522`), so pass the
      flag. The names are Rust `&str` literals in the **wasm data section**, not
      the JS glue:

```
cargo xtask build-csr --release
rg -ac 'jaunder\.boot\.' target/site/pkg/jaunder.wasm
```

      Expected: ≥ 1. (`-a` treats the binary as text; `target/site/pkg` is where
      `build-csr` writes — `build_csr.rs:42`.)

- [ ] **Step 7: Commit**

```bash
git add client/src/perf.rs client/src/lib.rs client/Cargo.toml csr/src/lib.rs
git commit -m "feat(csr): mark the boot phases so commit_to_mount can be decomposed (#794)"
```

---

### Task 10: Derived mount phases

Closes AC-6, AC-7, AC-8.

**Files:** Modify `capture-trace.ts`, `fixtures.ts`

- [ ] **Step 1: Harvest marks PER NAVIGATION, not at teardown.** `performance`
      marks and `PerformanceResourceTiming` entries are **per-document** —
      cleared on every full navigation. `readPagePerf` runs once at teardown
      (`fixtures.ts:656`), so it can only ever see the _last_ document's marks,
      while AC-8 requires **each** navigation to carry its breakdown. Snapshot
      inside the page-level `load` handler (Task 5 Step 2), keyed by the active
      navigation id, before the next document wipes them:

```ts
page.on("load", async () => {
  const id = activeNavigationId;
  if (id === null) return;
  marks.set(
    id,
    await page
      .evaluate(() =>
        performance
          .getEntriesByType("mark")
          .filter((m) => m.name.startsWith("jaunder.")) // PREFIX ONLY — never a name list
          .map((m) => ({ name: m.name, startTime: m.startTime })),
      )
      .catch(() => []),
  );
});
```

      Filter by prefix only. Enumerating known names would destroy the "adding a mark
      in Rust needs no TypeScript change" property (AC-6).

- [ ] **Step 2: Derive the pre-entry breakdown** (AC-8). Per navigation:
      `wasmFetchStartMs` / `wasmFetchMs` from the `.wasm`
      `PerformanceResourceTiming` entry (harvested in the same per-navigation
      snapshot); `wasmInstantiateMs` = `jaunder.boot.entry`'s mark time minus
      wasm fetch end. With the boot marks this decomposes `commit_to_mount` end
      to end.

- [ ] **Step 3: Derive `mount_to_settled_ms`** (AC-7) — D6's rule exactly. A
      mount-path request starts `>= navigation.committedMs`, finishes after
      `navigation.mountedMs`, and starts **before** the earlier of (a) the first
      timed action recorded after `mountedMs` or (b) the next navigation's
      `startedMs`. Result is the latest such request's end minus `mountedMs`;
      `null` when none qualifies. Every input exists today:
      `RequestRecord.startedMs/endedMs` (`fixtures.ts:39-48`),
      `NavigationRecord.startedMs/committedMs/mountedMs` (`:64-75`),
      `ActionRecord.startedMs` (`actions.ts:12-21`) — all Node-side
      `Date.now()`, one clock; actions are drained (`:648`) before the
      navigation summary is built (`:721`).

- [ ] **Step 4: Demonstrate prefix discovery** (AC-6). Temporarily add a mark in
      **Rust** whose name appears nowhere in TypeScript — e.g.
      `client::perf::mark("jaunder.boot.probe_ac6")` in `csr::main` — run
      `cargo xtask e2e sqlite chromium`, and confirm it appears in
      `e2e.boot_marks_json` with **no** TypeScript change. Then remove it and
      record the observation in the commit message. Injecting via
      `page.evaluate` would not demonstrate the claim.

- [ ] **Step 5: Verify against a real capture** (AC-7, AC-8). From the same run:
      navigations to `/app` report non-null `mountToSettledMs` covering the
      serialized session→timeline chain (`web/src/cockpit/component.rs:36`); a
      navigation with no post-mount fetch reports `null`, not `0`; **more than
      one** navigation carries boot marks (proving Step 1's per-navigation
      harvest, which a teardown-only harvest would fail).

- [ ] **Step 6: Commit**

```bash
git add end2end/tests/capture-trace.ts end2end/tests/fixtures.ts
git commit -m "test(e2e): derive the wasm-boot and post-mount-settle phases (#794)"
```

---

### Task 11: `traces analyze` span-coverage section

Closes AC-4's tooling half.

**Files:** Modify `xtask/src/lib.rs` (CLI), `xtask/src/traces/run.rs`,
`xtask/src/traces/parse.rs` (`mod tests` @ `:221`), `analyze.rs` (`:553`),
`render.rs` (`:299`), `testdata/otel-traces-sample.jsonl`

- [ ] **Step 1: Plumb the Playwright report through** — it is not reachable
      today. `traces analyze` takes only OTel JSONL (`lib.rs:314-317`),
      `analyze(&files,     filters)` has no report parameter, and
      `collect_trace_files` extracts only `capture/otel-traces.jsonl`
      (`run.rs:27`). Add a `--playwright-report <path>` arg (repeatable, like
      `files`) and have `traces run` pass the report it already has at
      `.xtask/diagnostics/e2e-<backend>-<browser>/playwright-report-<backend>.json`.
      Absent → the coverage section is skipped with a one-line note, never a
      silent zero.

- [ ] **Step 2: Extend the sample testdata** with one synthetic test's full
      lifecycle tree — `e2e.test.lifecycle` + `e2e.warmup` +
      `e2e.context_mint` + `e2e.test` + `e2e.page` + `e2e.teardown`, all
      carrying `e2e.project`; boot marks in `e2e.boot_marks_json` on **two**
      navigations; one navigation with non-null `mountToSettledMs` and **one
      with `null`**; a non-zero dropped count.

- [ ] **Step 3: Write the failing tests.** `parse.rs`: the tree parses; children
      resolve to the envelope by `parentSpanId`; boot marks round-trip; a `null`
      `mountToSettledMs` stays `None`, never `0`; a lifecycle span **without**
      `e2e.project` is dropped by a `--project` filter (pinning the Global
      Constraint). `analyze.rs`: coverage is an interval **union** — two
      overlapping children must not double-count — and the remainder is
      `playwright_duration - covered`, never negative.

- [ ] **Step 4:** Run
      `cargo nextest run --manifest-path xtask/Cargo.toml traces` → FAIL
- [ ] **Step 5: Implement.** The denominator is the Playwright report joined on
      test title + project — **not** `e2e.total_ms` (`fixtures.ts:770`), the
      span's own duration. Name the report path in the section header so the
      source is explicit.
- [ ] **Step 6:** Run
      `cargo nextest run --manifest-path xtask/Cargo.toml traces` → PASS
- [ ] **Step 7: Commit**

```bash
git add xtask/src/lib.rs xtask/src/traces/run.rs xtask/src/traces/parse.rs xtask/src/traces/analyze.rs xtask/src/traces/render.rs xtask/src/traces/testdata/otel-traces-sample.jsonl
git commit -m "feat(xtask): traces analyze reports per-test span coverage (#794)"
```

---

### Task 12: `docs/observability.md`

Closes AC-21, AC-22, AC-23.

- [ ] **Step 1** (AC-22): actions/navigations/resources are per-test **JSON
      attributes** on `e2e.test`; `request`, `storage.*`, `crypto.*`, `e2e.test`
      and the lifecycle spans are **spans**. Correct #788's "`action.timed`
      ×1233 spans" — 1233 was the entry count in `action_top_json`.
- [ ] **Step 2** (AC-21): Gecko implements no `longtask` PerformanceObserver, so
      the Firefox column is empty by engine limitation, not capture bug.
- [ ] **Step 3** (AC-23): the D2 hierarchy diagram; the D3 residual with Task
      13's measured number; the mark contract (`jaunder.` prefix discovery,
      unconditional, names owned by Rust alone); the `e2e.project` stamping
      requirement and why (`parse.rs:134`); and that `commit_to_mount` ends at
      `data-mounted` and so excludes the mount-path fetches, which
      `mount_to_settled_ms` covers. Link #801.
- [ ] **Step 4:** `prettier -w docs/observability.md`;
      `cargo xtask check --no-test` → PASS
- [ ] **Step 5: Commit**

```bash
git add docs/observability.md
git commit -m "docs(observability): trace shape, firefox long-tasks, and the lifecycle contract (#794)"
```

---

### Task 13: Measurement; record the floor

Closes AC-4's numeric half. Measures — does not implement.

- [ ] **Step 1:** Baseline — reuse Task 5 Step 6's saved capture, analyzed
      through the Task 11 section. Expected to reproduce #788's 28–31 %.
- [ ] **Step 2:** Branch — `cargo xtask traces run --top 25`; read the
      span-coverage section.
- [ ] **Step 3:** Write the measured residual into `docs/observability.md` as
      the D3 floor and into the spec's AC-4 as achieved. **No threshold was
      pre-committed.** If the residual is not substantially below 28–31 %,
      **stop and report** rather than adjusting the number to fit.
- [ ] **Step 4: Commit**

```bash
git add docs/observability.md docs/superpowers/specs/2026-08-01-issue-794-trace-coverage-gaps.md
git commit -m "docs(observability): record the measured per-test attribution floor (#794)"
```

---

## Coverage map

AC-1/2 → T6s3 · AC-3 → T5s6-7 + T6s5 · AC-4 → T11 + T13 · AC-5 → T9 · AC-6 →
T10s1,s4 · AC-7 → T10s3,s5 · AC-8 → T10s1-2,s5 · AC-9 → T9s6 · AC-10/11 → T4
(11c → T3s6) · AC-12 → T5 · AC-13 → T7s2 · AC-14 → T5s8 + T6s5 · AC-15 → T8 ·
AC-16/17 → T3s5-6,s8 · AC-18 → T3s7 · AC-19 → T3s1 (committed test) · AC-20 →
T3s6 · AC-21/22/23 → T12.

Two ACs are satisfied by measurement written back into the spec during execution
— AC-13's N (T7s2) and AC-4's figure (T13s3). Deliberate: both were left as
literals-to-be-measured rather than guessed.

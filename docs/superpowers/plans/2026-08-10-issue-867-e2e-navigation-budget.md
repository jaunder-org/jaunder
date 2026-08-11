# One boot per page — implementation plan (#867)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut the e2e suite's 211 navigations toward one document load per page,
and gate the result so it cannot drift back.

**Architecture:** The `registeredPage` fixture stops choosing an entry point and
becomes callable, so each test boots once at the URL it tests. Mid-test document
loads become in-app router navigation through a helper that keeps a
synchronisation barrier. A runtime budget counts document loads per Playwright
`Page` from `page.on("domcontentloaded")` — a signal no call site can bypass and
which same-document router pushes do not emit — and fails an undeclared second
load. A separate xtask static check keeps `page.goto` inside the navigation
wrapper.

**Tech Stack:** Playwright + TypeScript (`end2end/`), Rust (`xtask/`), the
`traces run` measurement harness.

Spec:
[`../specs/2026-08-10-issue-867-e2e-navigation-budget.md`](../specs/2026-08-10-issue-867-e2e-navigation-budget.md).
The spec is "what and why"; this plan is "how". ADR draft:
`docs/adr/drafts/e2e-one-boot-per-page.md` (already written; Task 10 only
re-checks it against what shipped).

---

## Review header

**Scope — in:** `end2end/tests/**`, one new xtask step, `CONTRIBUTING.md`,
`docs/observability.md`, the ADR draft, and one measurement campaign.

**Scope — out:** per-navigation cost (#864, #868, #869, #870), pre-warming of
any kind (ADR-0099), worker counts, the CI matrix, and fixing the secondary-page
attribution gap (filed in Task 1).

**Tasks:**

- **T1** — File the separable concern (secondary-page navigations unattributed).
- **T2** — Write the classification artifact; pins the predicted count before
  any measurement.
- **T3** — Add the boot budget module (`allowSecondBoot`, `domcontentloaded`
  counter) and its surfacing call in `goto`; armed explicitly, not yet
  automatically.
- **T4** — Add `navigateInApp`, the in-app navigation helper with a non-vacuous
  barrier.
- **T5** — Make `registeredPage` callable and migrate all 42 consumers.
- **T6a** — Convert `posts.spec.ts`: create and composer flows.
- **T6b** — Convert `posts.spec.ts`: edit, lifecycle and permalink flows.
- **T7** — Convert `profile.spec.ts`, `admin-site.spec.ts`, `backup.spec.ts`,
  the remaining specs and the navigating helpers; verify no assertion was lost.
- **T8** — Arm the budget automatically for every page; suite green under the
  budget.
- **T9** — Add the `e2e-goto-wrapper` xtask static check with markers on the 3
  raw sites.
- **T10** — Docs: `helpers.ts` docblock, `CONTRIBUTING.md`, ADR re-check.
- **T11** — Measure to the pre-registered protocol and write up
  `docs/observability.md`.

**Key risks / decisions:**

- **`framenavigated` would be wrong.** It fires for same-document router pushes
  too, so it would flag every conversion. `domcontentloaded` fires only on a
  real document load. This is the load-bearing choice in Task 3.
- **Task 5 is atomic and wide** — changing the fixture's shape breaks all 42
  consumers at once, so the migration lands in one commit. It is 42 judgment
  calls about entry paths, not 42 mechanical edits, and a wrong one yields a
  green-but-wrong suite rather than a compile error. The classification is what
  makes each call checkable.
- **Ordering:** violation _surfacing_ lands in Task 3 (its own tests need it);
  automatic _arming_ waits for Task 8, after the conversions. Arming earlier
  would fail the ~56 tests not yet converted or declared.
- **Barrier quality is the largest execution risk.** Tasks 6–7 replace `goto`'s
  unconditional `waitForMount` with `waitForURL` plus one selector, ~100 times.
  Task 4's already-matches assertion makes a vacuous barrier fail immediately
  rather than flake later — a mechanism, not advice.
- **Task 11 can fail its floor.** Per the spec that is a reportable outcome, not
  a revert: the idiom and gate still land.

---

## Global Constraints

Copied from the spec; every task's requirements implicitly include these.

- **ADR-0099:** nothing may warm a cache, reuse browser state across tests, or
  reintroduce a warmup at any scope. This work removes loads only.
- **ADR-0039:** per-test identity fixtures and the fresh-context-per-test model
  are untouched. No navigation that guarantees a clean starting state is
  removed.
- **ADR-0100:** `commitToMount` is Node-frame; used whole, never decomposed.
- **#887:** `wasmInstantiateMs` and per-segment attribution are not used to
  justify or evaluate this work.
- **ADR-0094 marker form:** `// e2e-goto-wrapper:allow <reason>` on the line
  immediately above the site. Line form only, reason required, one site per
  line, orphan markers fail, census derived and printed.
- **No `/s` (dotAll) regex flag in `end2end/`** — the suite's tsc target
  predates es2018 and `tsc` fails with TS1501. Use `[\s\S]` instead of `.`.
  (Found by the gate in Task 3.)
- **Per-navigation cost (from #866, for all arithmetic):** firefox 911 ms,
  chromium 689 ms `commitToMount`.
- **Commits:** run
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-867-e2e-nav-count -- cargo xtask check`
  before committing (`jaunder-commit`). **No `Co-Authored-By` trailer.**
- **Baseline to beat:** 211 test-attributed navigations + 20 secondary-page
  loads = 231 total document loads, 137 tests.

---

## File Structure

| File                                        | Responsibility                                                                                                                                                     |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `end2end/tests/bootBudget.ts`               | **new** — per-`Page` document-load counter and `allowSecondBoot`. Owns the budget; knows nothing about fixtures or navigation.                                     |
| `end2end/tests/navigate.ts`                 | **new** — `navigateInApp`, the in-app navigation barrier. Separate from `helpers.ts` because `helpers.ts` is the document-load surface and these are its opposite. |
| `end2end/tests/fixtures.ts`                 | `registeredPage` becomes callable; budget tracking attached at page setup.                                                                                         |
| `end2end/tests/helpers.ts`                  | `goto` unchanged in behaviour; docblock updated.                                                                                                                   |
| `end2end/tests/*.spec.ts`                   | Entry paths named; mid-test loads converted or declared.                                                                                                           |
| `xtask/src/steps/e2e_goto_wrapper_check.rs` | **new** — static check, modelled on `no_full_reload_check.rs`.                                                                                                     |
| `xtask/src/lib.rs:39,465,510`               | Register the new step alongside `no_full_reload_check`.                                                                                                            |
| `docs/superpowers/classification-867.md`    | The audit artifact (Task 2).                                                                                                                                       |
| `docs/observability.md`                     | The `#867` measurement section (Task 11).                                                                                                                          |

---

### Task 1: File the separable concern

**Files:** none in-tree — GitHub only.

**Interfaces:**

- Produces: an issue number, referenced in the spec's Separable Concerns section
  and in Task 2's artifact.

- [x] **Step 1: File the issue** via `jaunder-issues`. → **#895**

Title: `e2e: secondary-page navigations are unattributed and do not reconcile`

Body must state: 20 document loads sit on `e2e.page` spans carrying no
`navigation_top_json`; they do not reconcile with per-test totals
(`visibility.spec.ts` "Subscribers post: visible after Subscribe, hidden again
after Unsubscribe" reports 1 on its `e2e.test` span while its page spans sum to
5); so the headline 211 under-counts real page loads. Scope: attribution only —
ADR-0096 lineage. Labels `test-infra`, `observability`; milestone
`Observability & diagnostics`.

- [x] **Step 2: Cross-reference it**

Edit the spec's "Separable concerns" bullet to carry the new issue number.

```bash
git add docs/superpowers/specs/2026-08-10-issue-867-e2e-navigation-budget.md
git commit -m "docs(867): file the secondary-page attribution gap (#867)"
```

---

### Task 2: The classification artifact

Every document load classified before anything changes. This is the issue's
"breakdown by cause", its "recorded reason not to", the enumeration of which
pages lose incidental cold coverage, and the source of the pre-registered count.

**Files:**

- Create: `docs/superpowers/classification-867.md`

**Interfaces:**

- Consumes: the corpus at `~/measurements/jaunder/issue-866-preload/traces/`.
- Produces: **`PREDICTED_TOTAL`** — the post-change total document loads, cited
  by Task 11's pre-registration and by the spec's A9.

- [x] **Step 1: Derive the per-test navigation table from the corpus** —
      reproduced exactly: 137 tests, 211 test-attributed, 20 page-span, 231
      total, `dropped = 0`.

Use `ctx_execute` (javascript, absolute paths) over
`/home/mdorman/measurements/jaunder/issue-866-preload/traces/before-1-sqlite-chromium.jsonl`.
Sum `e2e.navigation_count` on `e2e.test` spans; take URLs from
`e2e.navigation_top_json`; separately sum navigations on `e2e.page` spans.
Assert `e2e.navigation_top_dropped == 0` on every span, so the URL lists are
complete rather than a top-N slice.

Expected: 137 tests, 211 test-attributed navigations, 20 page-span navigations.
If these do not reproduce, stop — the baseline is wrong and the rest of the plan
rests on it.

- [x] **Step 2: Write the classification table**

One row per navigation, grouped by file then test. Columns:
`file | test | url | class | reason`.

`class` is exactly one of:

- `removed` — the `registeredPage` boot of `/` that the test immediately leaves.
- `converted` — becomes in-app navigation; the destination's cold render is not
  the subject.
- `kept:entry` — the page's one legitimate boot.
- `kept:declared` — a second load on an already-booted page that stays. The
  `reason` column becomes the verbatim `allowSecondBoot` string.

Rules for assigning `converted` vs `kept:declared`, applied per the spec:

- The destination's cold render **is** the subject → `kept:declared`. Named
  subjects: permalink render (`posts.spec.ts` "published post renders at
  permalink", `unicode-slug.spec.ts` "Unicode-titled post reachable at
  permalink"), boot marks, flash/CLS probes.
- Re-reading state to prove persistence → `kept:declared` (`profile.spec.ts:27`,
  `admin-site.spec.ts`, `backup.spec.ts`).
- Otherwise, and a router push or a real UI control can reach it → `converted`.

- [x] **Step 3: Add the coverage-movement section** — one real loss: `/drafts`
      stops being an entry URL. No assertion deleted.

List every destination page that currently receives an incidental cold render
and will stop receiving one, with the test that provided it. This is the
enumeration the spec requires so coverage loss is stated, not asserted.

- [x] **Step 4: Compute and record the prediction** — `PREDICTED_TOTAL = 170`;
      saved 61; ceilings 55.6 s firefox / 42.0 s chromium; floor 33.3 s / 25.2
      s. Counts independently re-derived from the written table: 231 rows, and
      every per-file subtotal matches the trace.

```
PREDICTED_TOTAL = 231 - count(removed) - count(converted)
```

Record `count(removed)`, `count(converted)`, `count(kept:*)`, `PREDICTED_TOTAL`,
and the derived ceilings `count_saved × 911 ms` (firefox) and `× 689 ms`
(chromium) in a summary block at the top of the artifact. State explicitly that
this is registered before any timing arm is captured.

- [x] **Step 5: Commit**

```bash
git add docs/superpowers/classification-867.md
git commit -m "docs(e2e): classify all 231 document loads and pin the prediction (#867)"
```

**Also discovered here and filed, not folded in:**
[#896](https://github.com/jaunder-org/jaunder/issues/896) — `/posts/new` is
registered as a route but nothing in the app links to it, so the full composer
is reachable only by typing the URL.

---

### Task 3: The boot budget module

**Files:**

- Create: `end2end/tests/bootBudget.ts`
- Test: `end2end/tests/bootBudget.spec.ts`
- Modify: `end2end/tests/helpers.ts:67-79` (surface the violation from `goto`)

**Interfaces:**

- Produces:
  - `export function trackBoots(page: Page): void`
  - `export function allowSecondBoot(page: Page, reason: string): void`
  - `export function bootCount(page: Page): number`
  - `export function throwIfViolated(page: Page): void`

- [x] **Step 1: Write the failing tests**

`end2end/tests/bootBudget.spec.ts`. These run in the real browser because the
counter's whole claim is about which browser events fire. Import `test` from
`./fixtures`, not from `@playwright/test` — a bare Playwright `test` gets no
traceparent and no `attachTraceCapture`, so it would be the one untraced page in
the suite.

```ts
import { expect } from "@playwright/test";
import { test } from "./fixtures";
import { allowSecondBoot, bootCount, trackBoots } from "./bootBudget";
import { BASE_URL, goto } from "./helpers";

test("one document load counts one boot", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  expect(bootCount(page)).toBe(1);
});

test("a same-document router push does not count", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  await page.evaluate(() => history.pushState({}, "", "/app"));
  await page.waitForFunction(() => location.pathname === "/app");
  expect(bootCount(page)).toBe(1);
});

test("a second document load is rejected when undeclared", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  await expect(goto(page, "/login")).rejects.toThrow(
    /second document load [\s\S]*\/login[\s\S]*allowSecondBoot/,
  );
});

test("a declared second document load is permitted", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  allowSecondBoot(page, "the login page's cold render is the subject");
  await goto(page, "/login");
  expect(bootCount(page)).toBe(2);
});

test("an allowance is consumed, not permanent", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  allowSecondBoot(page, "one extra load");
  await goto(page, "/login");
  await expect(goto(page, "/register")).rejects.toThrow(/second document load/);
});

test("a raw page.goto is counted too", async ({ page }) => {
  trackBoots(page);
  // e2e-goto-wrapper:allow proves the counter does not depend on the wrapper
  await page.goto(`${BASE_URL}/`);
  await expect(goto(page, "/login")).rejects.toThrow(/second document load/);
});

test("an empty reason is rejected", async ({ page }) => {
  trackBoots(page);
  expect(() => allowSecondBoot(page, "   ")).toThrow(/reason/);
});
```

- [x] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-867-e2e-nav-count -- cargo xtask e2e-local bootBudget`

`e2e-local` (`xtask/src/lib.rs:174,601`) takes a single spec file and is the
iteration command throughout this plan; the full
`cargo xtask e2e <backend> <browser>` Nix suite is only for a task's final
verification.

Expected: FAIL — `./bootBudget` not found.

- [x] **Step 3: Implement against the tests**

Write `bootBudget.ts` to the four signatures above, **and** add the surfacing
call to `helpers.ts`'s `goto`: after `waitForMount`, call
`throwIfViolated(page)`. That must land here, not in Task 8 — three of the Step
1 tests assert `rejects.toThrow`, and without the surfacing they cannot pass. It
is safe to land now: only pages passed to `trackBoots` are watched, and nothing
arms it automatically until Task 8, so the rest of the suite is unaffected.

The design decisions the tests cannot express, and which must be honoured:

- Subscribe to **`page.on("domcontentloaded")`**, not `framenavigated`.
  `framenavigated` also fires for same-document `pushState` navigation, which
  would flag every conversion this plan makes; `domcontentloaded` fires only on
  a real document load. This is why the second test above exists.
- State lives in a module-level `WeakMap<Page, BudgetState>` so pages are not
  retained after their context closes.
- The rejection must surface where the test can see it. The `domcontentloaded`
  handler cannot itself reject the caller's promise, so record the violation in
  the state and have `throwIfViolated` — called from `goto` — throw on it. For a
  raw `page.goto` the violation surfaces on the next budget-aware call, which is
  what the "a raw page.goto is counted too" test exercises.
- Allowances are a counter, decremented per extra load, each carrying its reason
  for the failure message and for the derived census.

- [x] **Step 4: Run the tests, verify they pass**

Run: `... cargo xtask e2e-local bootBudget` Expected: PASS, 7 tests. **Actual: 7
passed (27.9 s)** — including "a same-document router push does not count",
which is what turns the `domcontentloaded` choice from reasoning into evidence.

One implementation decision the tests did not dictate, recorded because it is
load-bearing: the handler ignores `about:blank`. Playwright opens every page
there, and whether that fires `domcontentloaded` is an engine detail — counting
it would make every real entry look like a second load, plausibly on one engine
and not the other.

- [x] **Step 5: Commit**

```bash
git add end2end/tests/bootBudget.ts end2end/tests/bootBudget.spec.ts end2end/tests/helpers.ts
git commit -m "test(e2e): add a per-page document-load budget (#867)"
```

---

### Task 4: `navigateInApp`

**Files:**

- Create: `end2end/tests/navigate.ts`
- Test: `end2end/tests/navigate.spec.ts`

**Interfaces:**

- Produces:

```ts
export async function navigateInApp(
  page: Page,
  action: () => Promise<void>,
  expected: { url: string; ready: string },
): Promise<void>;
```

`action` performs the in-app move (usually a click on the real control).
`expected.url` is the destination path; `expected.ready` is the selector that
proves the destination route has rendered. Both are required — the barrier is
the point of the helper, and an optional `ready` would be omitted under time
pressure, which is the flake this task exists to prevent.

**`ready` must not already match before `action` runs.** A selector that is
already present is a vacuous barrier, and vacuous barriers are the failure mode
Tasks 6–7 would otherwise hit roughly a hundred times. `navigateInApp` asserts
this itself, so a bad selector fails immediately and loudly rather than
producing an intermittent test months later. This turns the plan's advice into a
mechanism.

**Selector note:** there is no `SEL.timeline` in `end2end/tests/selectors.ts` —
"timeline" does not appear there at all. Before writing these tests, read
`selectors.ts` and the existing `/app` inline-composer tests in `posts.spec.ts`
and use the selector those tests already use to prove the timeline rendered.
`DEST` below stands for that selector.

**Three deviations from the tests as drafted, each with its reason. Recorded
because they change what is verified, not merely how.**

1. **`DEST` resolves to `SEL.postBody`** — the inline composer's body field,
   present on `/app` and absent on `/`. `selectors.ts` has no timeline entry,
   and this is the selector the destination genuinely proves.
2. **The tests take `registeredPage`, not the bare `page`.** The `/app` nav item
   is authed-only (`web/src/sidebar/markup.rs`, `auth_required = true`), so an
   anonymous page has no `a[href="/app"]` to click. These become three more
   consumers for Task 5 to migrate.
3. **The span-recording test is dropped.** `drainActionsForTest`
   (`end2end/tests/actions.ts:79`) is destructive — it removes the records so
   `_autoPerfSpan` can finalise the span. A test that drained its own records
   would corrupt the very trace data this work is measured by. `navigateInApp`
   still wraps the move in `withTimedAction(page, "ui.navigate", …)`; that is
   verified by reading it, not by a test that breaks the fixture. Replaced by an
   assertion that no `domcontentloaded` fires during the move, which listens
   directly rather than through `bootBudget` — the budget is not armed
   automatically until Task 8, and a test proving the mechanism should not
   depend on it.

- [x] **Step 1: Write the failing tests**

```ts
import { expect } from "@playwright/test";
import { test } from "./fixtures";
import { drainActionsForTest } from "./actions";
import { bootCount, trackBoots } from "./bootBudget";
import { goto } from "./helpers";
import { navigateInApp } from "./navigate";

test("an in-app move changes route without a document load", async ({
  page,
}) => {
  trackBoots(page);
  await goto(page, "/");
  await navigateInApp(page, () => page.click('a[href="/app"]'), {
    url: "/app",
    ready: DEST,
  });
  expect(new URL(page.url()).pathname).toBe("/app");
  expect(bootCount(page)).toBe(1);
});

test("it fails loudly when the destination never renders", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  await expect(
    navigateInApp(page, () => page.click('a[href="/app"]'), {
      url: "/app",
      ready: "#never-rendered",
    }),
  ).rejects.toThrow();
});

test("it rejects a barrier that is already satisfied", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  await expect(
    navigateInApp(page, () => page.click('a[href="/app"]'), {
      url: "/app",
      ready: "body",
    }),
  ).rejects.toThrow(/already matches[\s\S]*barrier/);
});

test("it records timing in the trace like goto does", async ({
  page,
}, testInfo) => {
  trackBoots(page);
  await goto(page, "/");
  await navigateInApp(page, () => page.click('a[href="/app"]'), {
    url: "/app",
    ready: DEST,
  });
  // In-app moves must be as visible in traces as document loads are —
  // otherwise this work makes the suite less observable.
  const actions = drainActionsForTest(testInfo);
  expect(actions.map((a) => a.name)).toContain("ui.navigate");
});
```

`drainActionsForTest` is the real surface (`end2end/tests/actions.ts`) — spans
are Node-side records keyed by test, not a browser global. Read `actions.ts`
first and match its exact key argument and `ActionRecord` field names; the
`testInfo`/`.name` shape above is illustrative of the check, not of its
signature.

- [x] **Step 2: Run the tests, verify they fail**

Run: `... cargo xtask e2e-local navigate` Expected: FAIL — `./navigate` not
found.

- [x] **Step 3: Implement against the tests**

Signature as above. Wrap the whole move in
`withTimedAction(page, "ui.navigate", …)` so in-app moves appear in the trace
exactly as `page.goto` does. Sequence: assert `expected.ready` does **not**
currently match (throw naming the selector and saying the barrier would be
vacuous); run `action`; `page.waitForURL` against
`` `${BASE_URL}${expected.url}` ``; then
`waitForSelector(page, expected.ready)`. Do **not** call `waitForMount` — the
app is already mounted and `body[data-mounted]` is already present, so it would
pass vacuously and provide no barrier.

An optional `timeoutMs` was added to the destination options. The "never
renders" test would otherwise sit on Playwright's default timeout and risk
exceeding the ambient test budget; a caller that wants to fail fast needs to be
able to say so.

- [x] **Step 4: Run the tests, verify they pass**

Run: `... cargo xtask e2e-local navigate` Expected: PASS, 4 tests. **Actual: 3
passed (15.1 s)** — three, not four, because the span test was dropped for the
reason recorded above.

- [x] **Step 5: Commit**

```bash
git add end2end/tests/navigate.ts end2end/tests/navigate.spec.ts
git commit -m "test(e2e): add navigateInApp with a route-settled barrier (#867)"
```

---

### Task 5: `registeredPage` becomes callable

Atomic and wide: the fixture's shape changes, so all 42 consumers change in the
same commit.

**Files:**

- Modify: `end2end/tests/fixtures.ts:354` (the `registeredPage: Page;` entry in
  the fixture type — without this the change does not typecheck) and
  `fixtures.ts:476-488` (the fixture body)
- Modify: `end2end/tests/posts.spec.ts` (**31** sites),
  `end2end/tests/profile.spec.ts` (7), `end2end/tests/unicode-slug.spec.ts` (2),
  `end2end/tests/auth.spec.ts:212` (1), `end2end/tests/authed-flash.spec.ts:116`
  (1) — 42 total. `authed-cls.spec.ts:26` names `registeredPage` in a comment
  only and is **not** a consumer.

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces: `type RegisteredPage = (entry: string) => Promise<Page>`, exported
  from `fixtures.ts`. Tasks 6–8 rely on this name and signature.

- [x] **Step 1: Write the failing tests**

Append to `end2end/tests/bootBudget.spec.ts` (which already imports `test` from
`./fixtures`):

```ts
test("registeredPage boots at the given entry", async ({ registeredPage }) => {
  const page = await registeredPage("/posts/new");
  expect(new URL(page.url()).pathname).toBe("/posts/new");
});

test("registeredPage refuses a second call", async ({ registeredPage }) => {
  await registeredPage("/posts/new");
  await expect(registeredPage("/profile")).rejects.toThrow(
    /called twice[\s\S]*\/posts\/new/,
  );
});
```

Deliberately **no** `bootCount` assertion here: the fixture navigates before the
test body can call `trackBoots`, so counting only works once arming is automatic
— which is Task 8's deliverable and Task 8's test.

- [x] **Step 2: Run the tests, verify they fail**

Run: `... cargo xtask e2e-local bootBudget` Expected: FAIL — `registeredPage` is
a `Page`, not callable.

- [x] **Step 3: Implement the fixture**

Replace `fixtures.ts:483-488` with a fixture that keeps the seeding at
fixture-setup time and moves only the navigation to call time:

```ts
registeredPage: async ({ page, firstNav }, use) => {
  const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
  await applySeededSession(page.context(), record);
  let bootedAt: string | undefined;
  await use(async (entry: string): Promise<Page> => {
    if (bootedAt !== undefined) {
      throw new Error(
        `registeredPage() called twice: already booted at ${bootedAt}. ` +
          `A page boots once (#867); move within the app with navigateInApp, ` +
          `or declare a second load with allowSecondBoot.`,
      );
    }
    bootedAt = entry;
    await goto(page, entry, { timeout: firstNav });
    return page;
  });
},
```

Change the fixture type at `fixtures.ts:354` from `registeredPage: Page;` to
`registeredPage: RegisteredPage;`, and export
`export type RegisteredPage = (entry: string) => Promise<Page>;` from
`fixtures.ts` so Tasks 6–8 can name it.

Rewrite the fixture's docblock (`fixtures.ts:476-482`): it currently promises a
page "mounted at `/`" and cites "spec D8" as the reason. Both are now false.

Each site changes from `async ({ registeredPage: page }) => {` plus a first-line
`await goto(page, X)` to `async ({ registeredPage }) => {` plus
`const page = await registeredPage(X);`, where `X` is the test's real first
destination per Task 2's classification. Tests whose classification says their
`/` boot is `kept:entry` (an assertion runs at `/`) pass `"/"`.

- [x] **Step 4: Migrate all 42 consumers** — 45 sites, not 42: the three tests
      in `navigate.spec.ts` (added by Task 4) are consumers too. Zero
      `registeredPage: page` destructurings remain.

      Six tests now destructure `{ page, registeredPage }`. They seed through
      `createPostViaApi(page, …)` *before* their entry URL is known — in two
      cases the URL is derived from the seed result — and `page` is the same
      object the fixture later boots, with the session already on its context,
      so no extra navigation is implied.

      `published post renders at permalink` still loads `/posts/new` twice:
      its first navigation is inside `composePost` (`posts.ts:61`), which
      belongs to Task 6a. Net loads are unchanged, not worse. Flagged so it is
      not miscounted as a Task 5 saving.

- [x] **Step 5: Run the full suite, verify it passes**

Run: `... cargo xtask e2e sqlite chromium` then
`... cargo xtask e2e sqlite firefox` Expected: PASS. Navigation count should
already have fallen; do not measure it here — Task 11 owns measurement.

**Verified as `cargo xtask e2e-local` instead: 150 passed (6.8 m), exit 0** —
the whole suite under `chromium` + `chromium-admin`, on the host, no VM.

Why the substitution, recorded because it is a weaker check than the plan asked
for. The VM-based `e2e sqlite chromium` run failed at
`machine.wait_for_unit("otel-collector.service", timeout=60)` — the guest never
finished booting, so **no test executed**. Host load average was 40 on 16 cores,
because another session was running `e2e-postgres-firefox` and
`e2e-sqlite-firefox` from a different worktree. That is contention, not a defect
in this change, and re-running VM tests into a saturated host would have starved
that session too.

**What this leaves outstanding: firefox is unverified for Task 5.** The final
`cargo xtask validate` (Task 11 step 7) covers all four
`{sqlite,postgres}×{chromium,firefox}` combos and is the gate that must be green
before ship, so the coverage is deferred, not skipped. Later tasks should
re-attempt a VM run when the host is quiet rather than inheriting this
substitution by default.

- [x] **Step 6: Commit**

```bash
git add end2end/tests/fixtures.ts end2end/tests/*.spec.ts
git commit -m "test(e2e): registeredPage takes its entry path and boots once (#867)"
```

---

### Task 6a: Convert `posts.spec.ts` — create and composer flows

`posts.spec.ts` holds 90 of 211 navigations across 31 tests, each conversion a
behavioural rewrite rather than a mechanical edit. It is split in two so a
reviewer can reject one half while accepting the other.

This half: the create/composer group — the inline-composer tests (7, `/` →
`/app`), `create a post through the UI`, `save a draft through the UI`,
`create a post with a summary`, `create post with tags via UI`,
`over-long post summary shows inline error`, and the five `TagInput` tests.

**Files:**

- Modify: `end2end/tests/posts.spec.ts`, `end2end/tests/posts.ts`

**Interfaces:**

- Consumes: `navigateInApp` (Task 4), `RegisteredPage` (Task 5),
  `allowSecondBoot` (Task 3).

- [x] **Step 1: Convert each `converted` row in this group** — 2 rows, which is
      all the classification lists for this group (most of `posts.spec.ts`'s 19
      conversions are Task 6b's). Both go through the publish flash's permalink
      link: `create a post with a summary` (ready `article h1`) and
      `create post with tags via UI` (ready `.j-tag-list`). Both ready selectors
      were checked against the renderers (`web/src/posts/render.rs`,
      `web/src/taglist/markup.rs:18`) and are absent from the composer — the
      composer's tag UI is `.j-tag-chip`, a different class — so neither tripped
      the already-matches assertion.

      `composePost` (`posts.ts:61`) no longer navigates; it waits for
      `SEL.postBody` and fills in place, so its caller must already be on
      `/posts/new`. That wait stays: it asserts the new precondition, so a
      caller on the wrong page fails fast instead of filling nothing. This
      removes the double-load Task 5 deliberately left in
      `published post renders at permalink`.

Work test by test in classification order. Replace each `goto` with
`navigateInApp` driving the real control the app offers — the composer's publish
button, the permalink link in the publish flash, the tag chip. Where no control
exists, record that in the classification and reclassify the row as
`kept:declared`; do not synthesize a router push past a missing affordance.

`openComposer` (`end2end/tests/posts.ts:61`) navigates to `/posts/new`. Where a
test's entry already is `/posts/new` it no longer needs `openComposer`; where a
test reaches the composer mid-flow, `openComposer` becomes a `navigateInApp`
through the app's own compose control.

- [x] **Step 2: Add declarations for each `kept:declared` row in this group** —
      none exist in this group, so no `allowSecondBoot` calls were added.

`allowSecondBoot(page, "<reason from the classification>")` immediately before
the load it authorises, reason copied verbatim from the artifact.

- [x] **Step 3: Iterate**

Run: `... cargo xtask e2e-local posts` Expected: PASS.

- [x] **Step 4: Verify on both browsers** — **`e2e-local`: 150 passed (7.0 m)**,
      matching the pre-conversion baseline exactly. Firefox still deferred to
      the final `validate`, for the host-contention reason recorded in Task 5.

Run: `... cargo xtask e2e sqlite chromium` and
`... cargo xtask e2e sqlite firefox` Expected: PASS. New flake here is this
task's characteristic risk — an intermittent failure means the conversion lost a
barrier. Fix the barrier; never add a retry.

- [x] **Step 5: Commit**

```bash
git add end2end/tests/posts.spec.ts end2end/tests/posts.ts
git commit -m "test(e2e): move the posts composer flows to in-app navigation (#867)"
```

**One assumption to watch.** The permalink control is a plain `<a>`
(`web/src/posts/component.rs:773`), not leptos's `<A>`; the conversion relies on
`handle_anchor_click` intercepting it (ADR-0076). The pre-existing
`tag chip on permalink navigates to tag listing` test already rests on the same
fact for `.j-tag`. If the assumption is wrong, these two fail as an undeclared
second load once Task 8 arms the budget — loudly, not silently, which is the
point of arming it.

---

### Task 6b: Convert `posts.spec.ts` — edit, lifecycle and permalink flows

The remainder: the four 5-navigation tests,
`authenticated user can edit a draft post`,
`editing a published post freezes the slug`,
`editing a post updates tag chips and tag listing pages`,
`editing an invalid or nonexistent post shows not-found`,
`published post renders at permalink`,
`unpublishing from a permalink navigates to /drafts`,
`delete a draft from the drafts page`,
`scheduling a post shows a Scheduled-for badge`,
`tag chip on permalink navigates to tag listing`,
`user tag page lists that user's tagged posts`, and
`authenticated user can delete a published post`.

**Files:**

- Modify: `end2end/tests/posts.spec.ts`

**Interfaces:** as Task 6a.

- [x] **Step 1: Convert and declare, per the classification** — 14 conversions
      and 7 declarations. Permalink destinations use `ready: "article.j-post"`
      (emitted only by the post renderers), edit pages use `SEL.postBody`
      (absent from permalinks and `/drafts`); neither tripped the
      already-matches assertion. `/drafts` is reached through the real sidebar
      nav item (`web/src/sidebar/markup.rs:20`).

      **A defect in this plan surfaced here, not in the conversions.**
      `allowSecondBoot` threw on any page the fixtures had not armed — and
      automatic arming is Task 8, which cannot land until Task 7's files are
      converted, which cannot happen without declarations. A deadlock, and all
      6 failures in the first run were that one cause. Fixed in
      `bootBudget.ts`: a declaration can only ever follow a page's entry load,
      so arming late infers the entry rather than refusing it. Pinned by a new
      test, `a declaration works on a page armed late`.

      Also converted `scheduling from the edit page shows a Scheduled-for badge`
      (#863), which postdates the corpus and so appears in no classification
      row. Leaving it would have failed Task 8 with no task assigned to fix it.
      Its loads are outside the 231 baseline — see Amendment 1.

Same rules as Task 6a. Note the cold-render subjects concentrated here —
`published post renders at permalink` is `kept:declared` and its reason must say
the cold permalink render is the subject (this is the criterion A7 rests on).

`editing an invalid or nonexistent post shows not-found` loads two bad edit URLs
(`/posts/999999999/edit`, `/posts/abc/edit`). Both are cold loads of a not-found
route by design; classify accordingly rather than routing in-app.

- [x] **Step 2: Iterate**

Run: `... cargo xtask e2e-local posts` Expected: PASS.

- [x] **Step 3: Verify on both browsers** — **`e2e-local`: 151 passed (6.0 m)**,
      the 150 baseline plus the new late-arming regression test. The first run
      was 6 failed / 137 passed, every failure the `allowSecondBoot` deadlock
      above; fixed at the cause, not silenced. Firefox still deferred to the
      final `validate`.

- [x] **Step 4: Commit**

```bash
git add end2end/tests/posts.spec.ts
git commit -m "test(e2e): move the posts edit and permalink flows to in-app navigation (#867)"
```

---

### Task 7: Convert the remaining files

**Files:**

- Modify: `end2end/tests/profile.spec.ts`, `admin-site.spec.ts`,
  `backup.spec.ts`, `visibility.spec.ts`, `audiences.spec.ts`,
  `authed-flash.spec.ts`, `feeds.spec.ts`, `email.spec.ts`,
  `password_reset.spec.ts`, `unicode-slug.spec.ts`, `auth.spec.ts`, `helpers.ts`
  (`login:178`, `registerViaUi:245`, `requestPasswordReset:324`,
  `subscribeTo:339`, `unsubscribeFrom:354`, `followEmailLink:389`,
  `setAndVerifyEmail:299`)

**Interfaces:** as Task 6a.

- [x] **Step 1: Convert and declare, per the classification** — 2 conversions
      (the private-permalink click in `visibility.spec.ts`; the redundant
      `/login` `goto` in `password_reset.spec.ts` **deleted**, with a
      `waitForSelector` keeping the barrier the `goto` used to provide) and **36
      declarations**. `audiences.spec.ts` and `auth.spec.ts` needed no change —
      every row in both is `kept:entry`.

      **A hazard found here, and the rule it forced.** An `allowSecondBoot`
      allowance that is never consumed does not expire — it sits waiting and
      silently absorbs a later, genuine violation. So a declaration goes inside
      a helper only when the load is a second load for *every* caller
      (`setAndVerifyEmail`, `followEmailLink`); otherwise it belongs at the call
      site. `subscribeTo`/`unsubscribeFrom` forced the rule: one caller has a
      booted page, four pass a freshly created one where the same `goto` is that
      page's entry, so a helper-side declaration would dangle in four of five.
      Task 8 adds an orphan-allowance check so this stops depending on care.

      **One correction to the brief I gave.** I claimed `signInAsNewUser` boots
      the page, making `posts.spec.ts:382` a second load. It does not —
      `helpers.ts:214` seeds out of band with no navigation, so that `goto` is
      the page's entry, exactly as the classification has it. No declaration was
      added; one would have planted the dangling allowance described above.

The persistence reloads in `profile.spec.ts`, `admin-site.spec.ts` and
`backup.spec.ts` are `kept:declared` — they re-read through the server and are
the assertion. Each gets `allowSecondBoot(page, "…")` with the reason from the
artifact.

`password_reset.spec.ts:33-36` is the one clear redundancy: the test waits for
the router's own client-side redirect to `/login` and then issues a full `goto`
to `/login` anyway. The `goto` goes; the assertions run where the router landed.

The navigating helpers in `helpers.ts` each perform a `goto` on a page their
caller has usually already booted: `login` (`:178`, goto at `:185`),
`registerViaUi` (`:245`, goto at `:252`), `requestPasswordReset` (`:324`),
`subscribeTo` (`:339`), `unsubscribeFrom` (`:354`), `followEmailLink` (`:389`).
Convert each to take an already-booted page and move in-app, **or** document it
as a boot of a fresh page. Which applies is per caller; the classification
decides.

`login` and `registerViaUi` are the ADR-0098 holdouts whose subject _is_ the
real flow (`auth.spec.ts:14,52`; `authed-flash.spec.ts`;
`invite.spec.ts:57,94`). They keep their document load; what they need is a
declaration wherever the caller's page was already booted. Missing these is how
the budget fails at Task 8 with no step to fix it, so do not skip them.

- [x] **Step 2: Check no assertion was lost (spec A10)** — **no file fell.**
      Unchanged: admin-site 10, auth 28, authed-flash 20, backup 20, email 8,
      feeds 30, helpers 3, password_reset 8, posts 130, posts.ts 1, profile 18,
      unicode-slug 8, visibility 20. Counts rose only in the new files
      (bootBudget.spec 0→10, navigate.spec 0→4, navigate.ts 0→1). Four entries
      now sit under "Subject changes" in the classification.

For each file touched in Tasks 6a, 6b and 7, compare the count of `expect(`
occurrences against the fork point:

```bash
git diff wt-base-issue-867..HEAD -- end2end/tests --stat
rg -c 'expect\(' end2end/tests/*.spec.ts
git show wt-base-issue-867:end2end/tests/posts.spec.ts | rg -c 'expect\('
```

The per-file count must not fall. Where it does, either restore the assertion or
record the deliberate change. Write the list of tests whose subject changed —
what they exercised before and after — into
`docs/superpowers/classification-867.md` under a "Subject changes" heading; Task
11 quotes it into the write-up.

- [x] **Step 3: Run the full suite on both browsers** — **`e2e-local`: 151
      passed (5.0 m)**, green first time. Firefox still deferred to the final
      `validate`.

- [x] **Step 4: Commit**

```bash
git add end2end/tests docs/superpowers/classification-867.md
git commit -m "test(e2e): move the remaining specs to in-app navigation (#867)"
```

---

### Task 8: Wire the budget in

Lands after 6–7: wiring it earlier would fail every not-yet-converted test.

**Files:**

- Modify: `end2end/tests/fixtures.ts` (page setup),
  `end2end/tests/helpers.ts:67-79`

**Interfaces:**

- Consumes: `trackBoots` (Task 3).

- [x] **Step 1: Write the failing test** — four added, 14 in the file. Two for
      arming, two for the orphan check.

Append to `end2end/tests/bootBudget.spec.ts`:

```ts
test("the budget is armed for every test's page", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/");
  // No explicit trackBoots call — the fixture must have armed it.
  expect(bootCount(page)).toBe(1);
});

test("the budget is armed for a second page too", async ({
  registeredPage,
  tracedContext,
}) => {
  await registeredPage("/");
  const other = await (await tracedContext()).newPage();
  trackBoots(other); // must be idempotent — tracedContext already armed it
  await goto(other, "/");
  expect(bootCount(other)).toBe(1);
});
```

- [x] **Step 2: Run it, verify it fails**

Run: `... cargo xtask e2e-local bootBudget` Expected: FAIL — `bootCount` reports
0, the pages were never tracked.

**Added after Task 7: the orphan-allowance check.** An `allowSecondBoot`
allowance that is never consumed does not expire — it waits and silently absorbs
a later, genuine violation, which is the failure this whole budget exists to
prevent. Task 7 had to place every declaration by hand to avoid planting one.
Fix it structurally instead: at test teardown, fail if any tracked page has
unconsumed allowances, naming the page and the orphaned reasons. This is
ADR-0094's orphan-marker rule ("a marker whose site no longer exists fails") in
runtime form, and for the same reason — a written exemption nothing re-verifies
must at least be checked to still apply. Pin it with a test.

- [x] **Step 3: Arm the budget and surface violations** — `trackBoots` is the
      first statement of `_autoPerfSpan`'s body, and `tracedContext`'s `newPage`
      is wrapped so every page it opens is armed on creation (the same shape as
      the existing `close` wrapper) rather than editing 15 call sites. The
      orphan sweep runs unconditionally in the `finally` around `use()`, so a
      failing test cannot leak allowances into the next test in the worker, but
      the **throw** happens after the span export — a failing test never reaches
      it, so an orphan can never mask the real error.

Call `trackBoots(page)` in `fixtures.ts` at page setup, in the auto fixture that
already runs before the test body and owns per-page instrumentation
(`_autoPerfSpan`, `fixtures.ts:554`). Auto fixtures set up before requested ones
(`fixtures.ts:452-454`), so arming precedes `registeredPage`'s navigation —
which is what makes Task 8's first test possible and Task 5's impossible.

Arm every page the suite creates, not only the default one. The suite's only
`browser.newContext` is `fixtures.ts:420`, reached through `tracedContext`
(`fixtures.ts:546`), and all 15 spec-side `newPage()` calls sit on those
contexts — so wiring `trackBoots` into `tracedContext`'s page creation covers
them with no per-site edit. `trackBoots` must be idempotent, since a caller may
also arm explicitly.

The violation surfacing already landed in Task 3 (`throwIfViolated` called from
`goto`); nothing to add here.

- [x] **Step 4: Run the full suite on both browsers** — **`e2e-local`: 155
      passed, 0 failed, none skipped (4.6 m)**. Firefox still deferred to the
      final `validate`.

      The first run was **1 failed / 147 passed / 7 did not run**, and the one
      failure was the new orphan check firing correctly on its first outing. It
      caught an over-declaration in
      `owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app`,
      which declared two allowances where only one load lands — and in doing so
      corrected a wrong belief about the app. See Amendment 2 in the
      classification: the `/` load **never counts**, because the pre-paint
      `location.replace` (`web/src/app/render.rs`, ADR-0076's named exception)
      runs during head parsing and replaces the document before it reaches
      `DOMContentLoaded`; the load the budget sees is `/app`. Measured with a
      throwaway probe listening to both events, not reasoned.

      Without the orphan check that spare allowance would have sat in the queue
      and silently absorbed the next genuine undeclared second load on that page
      — the exact failure the budget exists to catch, hidden by the budget.

- [x] **Step 5: Commit**

```bash
git add end2end/tests/fixtures.ts end2end/tests/helpers.ts end2end/tests/bootBudget.spec.ts
git commit -m "test(e2e): enforce one boot per page (#867)"
```

---

### Task 9: The `e2e-goto-wrapper` static check

**Files:**

- Create: `xtask/src/steps/e2e_goto_wrapper_check.rs`
- Modify: `xtask/src/lib.rs:39` (module), `:465` and `:510` (both registration
  sites — the check must run in `check` and `validate` alike)
- Modify: `end2end/tests/layout-shift.ts:67`,
  `end2end/tests/authed-flash.spec.ts:142`, `:151` (markers)

**Interfaces:**

- Produces: `pub fn problems(scanned: &[(String, String)]) -> Option<String>`
  and `pub fn run(result: &mut CommandResult)`, mirroring
  `no_full_reload_check.rs:46,62`.

**Task 9 outcome.** Done; 12 unit tests (the 9 below plus block-comment prose,
census content, and the recovery line) and the full `cargo xtask check` green
with a 4-site census. Notes worth keeping:

- `files::with_extension` **is** extension-generic (`xtask/src/files.rs:23`
  takes `ext` as a parameter and already tests that `.mts` is not matched as
  `.ts`), so `"ts"` was the only change — no second walker.
- Site detection tracks `/* … */` state across lines, because the JSDoc
  paragraphs in `bootBudget.ts:19,29` discuss `page.goto` in prose and would
  otherwise read as sites. String literals are deliberately not tracked: that
  direction can only raise a false alarm, never miss a site.
- Unit tests run from `xtask/`, which is its own workspace —
  `cargo nextest run -p xtask` fails with package-not-found. Use
  `devtool run --cwd <worktree>/xtask -- cargo nextest run <filter>`.
- **Process miss, recorded rather than hidden:** tests and implementation were
  written in one pass, so no red run was observed for the 9. The only red seen
  was the package-not-found error, which is not the same thing.
- **A factual error in this plan and in the spec, found here.** Both claim the
  two `authed-flash.spec.ts` raw sites use `waitUntil: "commit"` through a
  pre-paint redirect. Only the first does. The second
  (`anonymous: /app bounces to /login`) uses `waitUntil: "domcontentloaded"`,
  and its redirect is **not** pre-paint — `PREPAINT_SCRIPT`
  (`web/src/app/render.rs:40`) only redirects `/`→`/app` for an authed marker,
  so the `/app`→`/login` bounce is CockpitPage's post-mount session gate. The
  wrapper's mount wait would probably succeed there, meaning the site may be
  convertible. It is marked, not converted: writing the plan's supplied reason
  would have put a false statement into a permanent, never-re-verified marker.
  Left as a residual opportunity, not chased.

- [x] **Step 1: Write the failing unit tests**

In `e2e_goto_wrapper_check.rs`, modelled on `no_full_reload_check.rs:90-148`:

```rust
#[cfg(test)]
mod tests {
    use super::{problems, violations};

    #[test]
    fn flags_a_raw_page_goto() {
        assert_eq!(violations("    await page.goto(url);\n"), vec![1]);
    }

    #[test]
    fn ignores_the_wrapper_call() {
        assert!(violations("    goto(page, \"/login\");\n").is_empty());
    }

    #[test]
    fn ignores_comment_lines() {
        assert!(violations("    // page.goto(url) is forbidden\n").is_empty());
    }

    #[test]
    fn a_marker_exempts_the_next_line() {
        assert!(
            problems(&[(
                "end2end/tests/x.ts".to_string(),
                "// e2e-goto-wrapper:allow the probe holds wasm so mount never completes\n\
                 await page.goto(url);\n"
                    .to_string()
            )])
            .is_none()
        );
    }

    #[test]
    fn a_bare_marker_fails() {
        let detail = problems(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow\nawait page.goto(url);\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("reason"));
    }

    #[test]
    fn an_orphan_marker_fails() {
        let detail = problems(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow stale\nawait goto(page, \"/\");\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("orphan"));
    }

    #[test]
    fn two_sites_on_one_marked_line_fail() {
        let detail = problems(&[(
            "end2end/tests/x.ts".to_string(),
            "// e2e-goto-wrapper:allow one reason\n\
             await page.goto(a); await page.goto(b);\n"
                .to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("exactly one"));
    }

    #[test]
    fn the_helpers_module_may_call_page_goto() {
        // The wrapper itself is not a bypass.
        assert!(super::is_exempt_path("end2end/tests/helpers.ts"));
    }

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(
            problems(&[(
                "end2end/tests/x.ts".to_string(),
                "    await goto(page, \"/\");\n".to_string()
            )]),
            None
        );
    }
}
```

- [x] **Step 2: Run them, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-867-e2e-nav-count -- cargo nextest run -p xtask e2e_goto_wrapper`
Expected: FAIL — module does not exist.

- [x] **Step 3: Implement against the tests**

Signatures: `fn violations(source: &str) -> Vec<usize>`,
`fn is_exempt_path(path: &str) -> bool`,
`pub fn problems(scanned: &[(String, String)]) -> Option<String>`,
`pub fn run(result: &mut CommandResult)`.

Follow `no_full_reload_check.rs` exactly in shape: per-line matching (with the
same stated limitation in the module doc), `POLICED_ROOTS = &["end2end/tests"]`,
a missing root is a hard failure. `is_exempt_path` exempts only
`end2end/tests/helpers.ts` — the wrapper's own home.

`no_full_reload_check.rs:65` calls `files::with_extension(root, "rs")` — the
extension is a parameter, so `"ts"` is the only change needed _if_ that helper
is genuinely extension-generic. Read `xtask/src/files.rs` and confirm before
assuming it; its doc comment at `no_full_reload_check.rs:59` says "every Rust
file", which may reflect a Rust-specific walk. If it is Rust-specific, extend
the helper rather than copying a second walker.

Per ADR-0094 the check must additionally **derive and print the census** of live
markers (`file:line — reason`) on success, because the exemption population is
un-recheckable. The failure detail must carry a `recovery:` line pointing at
`goto` in `helpers.ts`.

- [x] **Step 4: Run the unit tests, verify they pass**

Expected: PASS, 9 tests.

- [x] **Step 5: Register the step and mark the three raw sites**

Add `pub mod e2e_goto_wrapper_check;` at `xtask/src/lib.rs:39` and
`steps::e2e_goto_wrapper_check::run(&mut result);` beside the existing
`no_full_reload_check` calls at `:465` and `:510`.

Markers, each on the line immediately above its site:

- `end2end/tests/layout-shift.ts:67` —
  `// e2e-goto-wrapper:allow the CLS probe holds wasm so mount never completes; the wrapper's waitForMount would hang`
- `end2end/tests/authed-flash.spec.ts:142` and `:151` —
  `// e2e-goto-wrapper:allow waitUntil "commit" plus waitForURL through the pre-paint redirect; the wrapper would wait on the wrong thing`

- [x] **Step 6: Run the gate, verify it passes**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-867-e2e-nav-count -- cargo xtask check`
Expected: PASS, with the marker census printed.

- [x] **Step 7: Commit**

```bash
git add xtask/src/steps/e2e_goto_wrapper_check.rs xtask/src/lib.rs end2end/tests
git commit -m "feat(xtask): add the e2e-goto-wrapper static check (#867)"
```

---

### Task 10: Documentation

**Files:**

- Modify: `end2end/tests/helpers.ts:4-38`, `CONTRIBUTING.md`
- Modify: `docs/adr/drafts/e2e-one-boot-per-page.md` (re-check only)

- [x] **Step 1: Update the `helpers.ts` usage-rules docblock**

It currently encodes the pre-#867 contract — "Always use `goto`" with no mention
of a budget. Add: a page boots once at the URL under test; move within the app
with `navigateInApp`; a second document load needs
`allowSecondBoot(page, reason)`; `page.goto` outside this module fails the
`e2e-goto-wrapper` check. Keep the existing rules that still hold.

- [x] **Step 2: Update `CONTRIBUTING.md`**

Add the one-boot-per-page rule to its e2e testing section, pointing at the ADR
and at `helpers.ts` for the API.

- [x] **Step 3: Re-check the ADR draft against what shipped**

Read `docs/adr/drafts/e2e-one-boot-per-page.md` against the implemented code and
correct anything that drifted during 3–9. Do **not** number it —
`cargo xtask adr promote` does that at ship.

- [x] **Step 4: Commit**

```bash
git add end2end/tests/helpers.ts CONTRIBUTING.md docs/adr/drafts/e2e-one-boot-per-page.md
git commit -m "docs(e2e): document the one-boot-per-page rule (#867)"
```

---

### Task 11: Measure and write up

**Files:**

- Modify: `docs/observability.md` (new `## #867` section)

**Interfaces:**

- Consumes: `PREDICTED_TOTAL` and the ceilings from Task 2.

- [x] **Step 1: Confirm the pre-registration is already committed**

Task 2's artifact must be in git history **before** the first arm is captured.
Verify with `git log --oneline -- docs/superpowers/classification-867.md`. If it
is not, stop: the prediction would be post-hoc and the measurement worthless.

- [x] **Step 2: Capture the deciding arms**

Single-worker, sqlite × {chromium, firefox}, 3 runs per arm, `before` and
`after` interleaved run-by-run, distinct `e2eSalt` per run, `retries` above 0 so
`flaky` is observable. Quiesce the host first. Use Bash background mode — this
is a long cold run.

The `before` arm is the branch's fork point, `wt-base-issue-867`.

- [ ] **Step 3: Capture the confirming arms** — **NOT RUN.** Gate-settings
      (2-worker) arms were not collected. The spec gives this set no pass
      criterion — it is reported, not gated — so the verdict does not depend on
      it, but the gate-time figure the issue quotes ("65 s off the gate") is
      therefore still unmeasured. The deciding set is single-worker, and
      `playwright.config.ts:50` ties `fullyParallel` to the worker count, so the
      single-worker saving does not translate linearly. Left undone
      deliberately, not overlooked.

Gate settings (2 workers), same shape. Reported only — no pass criterion, per
the spec.

- [x] **Step 4: Certify the corpus before analysing**

Confirm `dropped = 0` and a full mark set on every mounted navigation, as #818
and #836 did. An uncertified corpus is not analysed.

- [x] **Step 5: Evaluate against the pre-registration**

Three checks, in order:

1. **Count.** Post-change total document loads == `PREDICTED_TOTAL`, exactly. A
   miss means the change did not do what the classification said; investigate
   before reading any timing.
2. **Floor.** Realised suite-wall-clock saving ≥ 60% of the ceiling in **both**
   engines.
3. **Guardrail.** Summed `flaky + unexpected` == 0 across each browser's three
   runs.

- [x] **Step 6: Write `docs/observability.md`**

A `## #867 — removing navigations` section carrying: the method (deciding and
confirming sets, and why single-worker decides), the pre-registered prediction
and ceilings quoted from the artifact, the realised per-arm suite wall-clock,
the count check, the floor verdict, the flake guardrail, the coverage-movement
list, and the "Subject changes" list from Task 7 step 2 (spec A10).

**If the floor is missed**, write it up as the negative result the spec
pre-authorised: the idiom and gate stay, the performance claim fails, and the
residual — navigations removed but wall-clock not recovered — is investigated
and filed as its own issue. Do not soften the finding; #866's value came from
failing honestly.

- [x] **Step 7: Run the full gate**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-867-e2e-nav-count -- cargo xtask validate`
Expected: PASS, all four `{sqlite,postgres}×{chromium,firefox}` combos.

- [x] **Step 8: Commit**

```bash
git add docs/observability.md
git commit -m "docs(observability): measure the #867 navigation reduction (#867)"
```

---

## Code review at the deliverable boundary (after Task 10)

A two-axis review ran over `wt-base-issue-867...HEAD` — Standards and Spec, in
parallel sub-agents. It found one real hole in the headline mechanism and one
real defect, both since fixed.

**A3 was partial — detection without enforcement.** The budget _detected_ every
document load (per-`Page` event, unbypassable) but only _raised_ from `goto`. A
test taking an undeclared second load and issuing no later `goto` had its
violation recorded and discarded — and the raw sites carrying static markers are
exactly the ones that never route through `goto`, so the gap lined up precisely
with the sites least covered otherwise. Fixed: teardown now raises violations as
well as orphans, through one sweep. Closing it surfaced **no** hidden violation
(156 passed), so the conversions were sound; the gap was in plumbing.

**A defect in late arming.** `allowSecondBoot` on an unarmed page pushed
`page.url()` unconditionally, recording `about:blank` as the entry — after which
the page's first real load would consume the allowance and leave a genuine
second load uncounted. The budget disarming itself. Fixed: infer an entry only
from a real document.

**Three smaller items, all taken.** `consumedReasons` (and the state field it
read) deleted as speculative generality; the whole-file gate exemption for
`helpers.ts` replaced with an ordinary ADR-0094 marker, since ADR-0085 principle
4 forbids region-scoped exemptions and that file holds exactly one `page.goto`;
and the check now walks each file once instead of four times.

**Deviation from this plan, accepted.** Task 9 specified `pub fn problems`,
copying `no_full_reload_check.rs`. Collapsing the four scans made `problems` /
`census` / `violations` thin test-facing wrappers over `audit`, so they are now
`#[cfg(test)]`. Preserving the signature would have meant keeping the redundant
scan the review asked us to remove — a plan written before the code existed does
not outrank the code.

**Scope creep, flagged honestly by the reviewer and kept.** The orphan-allowance
rule is a failure mode the spec never named. It is justified — it caught a real
over-declaration — but it is added scope, and Task 11's write-up should say so
rather than present it as something the spec asked for.

**One soft spot left for Task 11.** Amendment 1 says the #863 test's loads must
be subtracted before comparing observed to predicted, but never pins the number.
Pin it _before_ the first arm is captured.

## Self-review

**Spec coverage.** A1→T5 (incl. the `fixtures.ts:354` type), A2→T5/T2, A3→T3/T8,
A4→T3, A5→T9, A6→T6a/T6b/T7 (T7 explicitly covers `login` and `registerViaUi`),
A7→T2/T6b, A8→T2, A9→T11, A10→T7 step 2 + T11 step 6, A11→T4, A12→T10, A13→T11
step 7, A14→T11, A15→T10. Separable concerns→T1. No spec section is unclaimed.

**Placeholder scan.** Four steps defer content to the classification rather than
inlining it (T6a step 1, T6b step 1, T7 step 1, T9 step 5's reasons). That is
deliberate and not a placeholder: the classification is itself a numbered,
committed task whose output those steps consume, and inlining 231 rows here
would duplicate it. Two steps name a file to read rather than an API to call —
T4's `DEST` selector (`selectors.ts` has no `timeline`) and its
`drainActionsForTest` shape (`actions.ts`), and T9's `files::with_extension`
genericity (`xtask/src/files.rs`). Each names the exact file and what to look
for; these are real dependencies on existing code, not deferred decisions.

**Type consistency.** `RegisteredPage = (entry: string) => Promise<Page>` (T5,
exported from `fixtures.ts`) is what T6a–T8 consume. `trackBoots` /
`allowSecondBoot` / `bootCount` / `throwIfViolated` (T3) are used under those
names in T4, T5, T6a, T6b, T7, T8. `navigateInApp(page, action, {url, ready})`
(T4) is used with that shape in T6a, T6b and T7. `problems` / `run` /
`violations` / `is_exempt_path` (T9) match `no_full_reload_check.rs:46,62`'s
exported shape.

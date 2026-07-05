# Plan — issue #260: reduce E2E test boilerplate

**Spec:**
[`docs/superpowers/specs/2026-07-05-issue-260-e2e-reduce-boilerplate.md`](../specs/2026-07-05-issue-260-e2e-reduce-boilerplate.md)
(the "what/why"; this plan is the "how" — read the spec's §1–§6 + acceptance
criteria alongside). **Issue:**
[#260](https://github.com/jaunder-org/jaunder/issues/260) / sub-issues
#261–#266. **For agentic workers:** drive with **`jaunder-iterate`**, delegating
a task to a subagent via **`jaunder-dispatch`** when useful; commit via
**`jaunder-commit`**.

---

## Review header

**Goal.** Extract shared e2e helpers and make timeout scaling ambient so the
Playwright suite is more concise and consistent, changing no test's assertions
and (beyond the spec §1 ambient-budget rise) no test's timeout.

**Scope — in:** `end2end/tests/` shared modules (`fixtures.ts`, new
`selectors.ts`, new `posts.ts`, `helpers.ts`, `mail.ts`) and the spec call sites
that migrate onto them. **Out:** right-sizing timeout budgets (Task 1 files it
as a follow-up), splitting `fixtures.ts`, a `helpers/` subdirectory, one-off
selectors, changes to the `slowBrowser*` math.

**Tasks:**

1. File the "right-size e2e timeout budgets from measured durations" follow-up
   issue.
2. `selectors.ts` — `SEL` constants + migrate high-leverage selector literals
   (#263).
3. Ambient-timeout fixture surface in `fixtures.ts` (no spec migration yet)
   (#261).
4. Migrate spec timeout call sites onto the ambient surface (#261).
5. `posts.ts` — `createPostViaApi` + `composePost` + adopt (#262).
6. `fillLoginForm` in `helpers.ts` + login refactor + timed-`click()` routing
   (#264).
7. `extractToken` in `mail.ts` + collapse 3 sites (#265).
8. `expectFlash` — opportunistic; adopt or leave #266 open with a note (#266).
9. Full-gate `cargo xtask validate` + acceptance-criteria sweep.

**Key risks / decisions.**

- **First-nav budget lowering (spec §1).** `firstNav` is the modal `10_000`
  only; adopt it **exclusively** at sites whose first-nav budget is exactly
  `10_000`. `posts.spec.ts:196` (`12_000`), `visibility` (`20_000`),
  `feeds`/`atompub`/`media` (`30_000`) keep explicit budgets. Task 4 guards this
  with a grep.
- **Auto-fixture ordering** — Playwright runs auto fixtures before requested
  ones, so the ambient budget covers `verifiedUser`/`registeredPage` setup; Task
  3 relies on this and retires `verifiedUser`'s hand-rolled `setTimeout`.
- **`setTestBudget` placement** — must be the first body line, before any
  awaited body setup (Task 4).
- **Chromium-only fast loop** — `e2e-local` runs chromium + chromium-admin;
  Firefox/WebKit timeout-scaling parity is only proven by the final `validate`
  (Task 9). Per-task smoke is necessary but not sufficient.

---

## Global constraints

- **Behaviour-preserving.** No test's assertions change. The only timeout change
  is the spec-§1 ambient rise for ≤30 000 whole-test budgets. **No first-nav
  budget changes at all.**
- **Wrapper discipline** (`helpers.ts` module doc): use `goto`/`click`/
  `waitForSelector`; never `page.goto`/`page.click`/`page.waitForSelector` raw;
  never `waitForLoadState("networkidle")`; keep `waitForNewEmail(previousCount)`
  snapshot-before-action and the recipient-scoped `mailbox` semantics.
- **New modules are flat siblings** of the existing `end2end/tests/*.ts`
  modules.
- **Verification ladder.** Per task: `cargo xtask e2e-local <spec.spec.ts>`
  (fast, chromium; **requires a dev server on :3000** — start
  `cargo leptos end-to-end`, and before trusting a result confirm the server is
  _this_ worktree's, not a stale leftover: check `ss -ltn 'sport = :3000'`) plus
  the task's `rg` acceptance greps. Final gate (Task 9): `cargo xtask validate`
  (all four `{sqlite,postgres}×{chromium,firefox}` combos).
- **Commit** (`jaunder-commit`): run the repo TS format/lint that
  `cargo xtask check` covers, then `cargo xtask check` clean, before each
  commit. **No `Co-Authored-By` trailer.** One commit per task (or per coherent
  sub-step), referencing the sub-issue it closes.
- **This is a test refactor, not TDD product code** — the specs themselves are
  the tests; "red→green" is "suite stayed green across the migration," verified
  by the ladder above, not by adding new test cases.

---

## Task 1 — File the timeout-right-sizing follow-up issue

Separable concern (spec Non-goals). File via **`jaunder-issues`** before
touching code so it can be picked up independently.

- **Title:**
  `E2E: right-size e2e timeout budgets from measured results.json durations`
- **Body:** Now that timeout scaling is ambient (#261), the surviving explicit
  budgets (the 13 `setTestBudget` sites at 60/90/150 s) and the
  `DEFAULT_TEST_BUDGET_MS` default may be inflated. The Playwright JSON reporter
  already writes per-test `duration` to `end2end/test-results/results.json` on
  every run (all four combos). Collect durations across combos at representative
  `workers`, compare to budgets, and lower over-provisioned budgets. **Risk:**
  lowering a budget can flake under worker contention — must be validated across
  all combos, not chromium-only. Label `test-infra`, milestone `E2E test suite`,
  parent #260.
- **Verify:** issue created; record its number here and reference it in Task 9 /
  the PR body.

**Done when:** `[x]` follow-up issue filed and its number recorded in this plan.
→ Filed as **#270** (https://github.com/jaunder-org/jaunder/issues/270), linked
as sub-issue of #260.

---

## Task 2 — `selectors.ts` + migrate high-leverage selectors (#263)

Foundational: Tasks 5 and 6 reference `SEL`.

**Files:**

- **New** `end2end/tests/selectors.ts` — export `SEL`:
  ```ts
  export const SEL = {
    saveSummary: ".j-save-summary",
    postBody: 'textarea[name="body"]',
    publishButton: (value: string) =>
      `button[name="publish"][value="${value}"]`,
    error: ".error",
    submit: 'button[type="submit"]',
    logoutLink: 'a[href="/logout"]',
    username: 'input[name="username"]',
    password: 'input[name="password"]',
    topbarHeading: ".j-topbar h1",
  } as const;
  ```
  (Note the logout link normalises to double quotes — the suite currently uses
  the single-quoted `a[href='/logout']`; the selector text is identical.)
- Edit spec files that literal these selectors (`posts`, `visibility`,
  `unicode-slug`, `auth`, `password_reset`, `admin-site`, `email`, `feeds`,
  `authed-flash`, `atompub`, `media`, and the `helpers.ts`/`fixtures.ts` uses of
  the auth trio) to import and use `SEL.*`.

**Interfaces:** none beyond `SEL`. Leave one-off selectors
(`input[name="email"]`, `input[name="slug_override"]`, `#audience-base`,
`.j-composer`, etc.) inline, and leave **compound** selectors that embed a `SEL`
string (e.g. `'.j-app-passwords button[type="submit"]'` in `atompub`) inline.

**Verify:**

- `rg` shows no **standalone** literal of the migrated selectors in `*.spec.ts`
  (compound/one-off matches expected — AC5).
- `cargo xtask e2e-local` (full chromium pass) green — this touches many files,
  so run the whole suite, not one spec.

**Done when:** `[ ]` `selectors.ts` created, high-leverage literals migrated,
suite green.

---

## Task 3 — Ambient-timeout fixture surface in `fixtures.ts` (#261, no spec migration)

Add the surface first, migrate call sites in Task 4 — this keeps a green
intermediate (existing explicit `test.setTimeout` calls still override the new
ambient default, so behaviour is unchanged until Task 4).

**Files — `end2end/tests/fixtures.ts`:**

- Add `export const DEFAULT_TEST_BUDGET_MS = 30_000;`
- Add the auto timeout fixture to the `base.extend<{…}>({…})` object:
  ```ts
  _autoTestTimeout: [
    async ({}, use, testInfo) => {
      testInfo.setTimeout(slowBrowserTimeoutMs(testInfo, DEFAULT_TEST_BUDGET_MS));
      await use();
    },
    { auto: true },
  ],
  ```
  (declare `_autoTestTimeout: void` in the fixtures type param.)
- Add
  `export function setTestBudget(chromiumBudgetMs: number): void { const info = test.info(); info.setTimeout(slowBrowserTimeoutMs(info, chromiumBudgetMs)); }`
- Add the `firstNav` fixture value:
  ```ts
  firstNav: async ({}, use, testInfo) => {
    await use(slowBrowserFirstNavigationTimeoutMs(testInfo, 10_000));
  },
  ```
  (type `firstNav: number`.)
- Add the `registeredPage` fixture (fresh context + page, `register` with the
  scaled 10 000 first-nav budget, `use(page)`, close context on teardown — model
  on the existing `user` fixture but yield the `page` instead of credentials):
  ```ts
  registeredPage: async ({ browser }, use, testInfo) => {
    const context = await browser.newContext();
    const page = await context.newPage();
    await register(page, slowBrowserFirstNavigationTimeoutMs(testInfo, 10_000));
    await use(page);
    await context.close();
  },
  ```
  (type `registeredPage: Page`.)
- **Retire** `verifiedUser`'s
  `testInfo.setTimeout(slowBrowserTimeoutMs(testInfo, 30_000));` (line ~271) and
  replace its multi-line load-bearing comment with a one-line pointer: setup
  coverage now comes from the ambient `_autoTestTimeout` auto fixture.
- Route `verifiedUser`'s raw `page.click('button[type="submit"]')` (line ~278)
  through `click(page, SEL.submit)`; **add `click` to the `./helpers` import**
  (currently `{ goto, login, register }`) and import `SEL` from `./selectors`.
- Export the new symbols (`DEFAULT_TEST_BUDGET_MS`, `setTestBudget`) alongside
  the existing `export { expect, test }`.

**Verify:** `cargo xtask e2e-local` full chromium pass green (no spec migrated
yet; the ambient default is ≥ every existing explicit budget so nothing
tightens).

**Done when:** `[ ]` fixture surface added, `verifiedUser` line retired + its
click routed through `click()`, suite green.

---

## Task 4 — Migrate spec timeout call sites (#261)

**Files — every `*.spec.ts` with a timeout ritual** (`posts`, `visibility`,
`atompub`, `auth`, `authed-flash`, `media`, `password_reset`, `unicode-slug`,
`admin-site`, `email`, `feeds`):

- **Drop** every `test.setTimeout(slowBrowserTimeoutMs(testInfo, N))` /
  `info.setTimeout(slowBrowserTimeoutMs(info, N))` with `N ≤ 30_000` (inherits
  the ambient default). Drop the now-unused `testInfo` param / `const info = …`
  where it becomes dead.
- **Convert** the 13 `N > 30_000` sites to `setTestBudget(N)` as the **first
  body line** (before any awaited setup): `feeds` (60/150/90/60/60),
  `visibility` (60/60/90/90), `atompub` (60/60/90), `posts:680` (60). Import
  `setTestBudget` from `./fixtures`.
- **Adopt `firstNav`** at **every** exactly-`10_000` first-nav site: replace the
  `slowBrowserFirstNavigationTimeoutMs(testInfo, 10_000)` recompute with the
  `firstNav` fixture value. This includes the username-_capturing_ sites
  (`posts` 290/338/345/395/402, `authed-flash:42`) — they keep an explicit
  `register(page, firstNav)` (now using the fixture value) but do **not**
  collapse to `registeredPage`.
- **Adopt `registeredPage`** only where a test opens with a bare
  `register(page, firstNav)` and **discards** the username — switch to the
  `registeredPage` fixture and drop the preamble.
- **Leave entirely** the non-`10_000` first-nav sites — `posts.spec.ts:196`
  (`12_000`), `visibility` (`20_000`), `feeds`/`atompub`/`media` (`30_000`):
  keep their explicit `slowBrowserFirstNavigationTimeoutMs(testInfo|info, N)`.

**Verify (AC1–AC3):**

- `rg 'setTimeout\(slowBrowserTimeoutMs' end2end/tests/*.spec.ts` → **0**.
- `rg 'setTestBudget\(' end2end/tests/*.spec.ts` → exactly 13 sites, all
  `N > 30_000`.
- `rg 'slowBrowserFirstNavigationTimeoutMs\(testInfo, 10_000\)' end2end/tests/*.spec.ts`
  → **0**.
- `rg -P 'slowBrowserFirstNavigationTimeoutMs\((testInfo|info),\s*(12_000|20_000|30_000)\)'`
  still present (unchanged; `-P`/alternation because atompub uses the `info`
  variable, not `testInfo`).
- `cargo xtask e2e-local` full chromium pass green.

**Done when:** `[ ]` all timeout sites migrated per the greps, suite green.

---

## Task 5 — `posts.ts` post-creation helpers + adopt (#262)

**Files:**

- **New** `end2end/tests/posts.ts`:

  ```ts
  export async function createPostViaApi(
    page: Page,
    opts: {
      body: string;
      tags?: string[];
      publish?: boolean;
      slug?: string | null;
    },
  ): Promise<{ post_id: number; permalink: string }>;
  export async function composePost(
    page: Page,
    opts: { body: string; summary?: string; slug?: string; publish: boolean },
  ): Promise<Locator /* SEL.saveSummary */>;
  ```

  - `createPostViaApi`: POST `${BASE_URL}/api/create_post` with
    `format: "markdown"`, `slug_override: opts.slug ?? null`,
    `publish: opts.publish ?? true`, `tags` forwarded;
    `expect(res.ok(), \`create_post failed (\${res.status()}):
    …\`).toBeTruthy()`; return parsed typed JSON. Model on `feeds.spec.ts`'s local `publishPost`.
  - `composePost`: `goto(page, "/posts/new")` → fill `SEL.postBody` (+
    summary/slug inputs when provided) →
    `click(page, SEL.publishButton(opts.publish ? "true" : "false"))` (the
    publish button's `value` is the boolean string `"true"`/`"false"`, e.g.
    `button[name="publish"][value="true"]` — confirm against a current
    `/posts/new` site) → `waitForSelector(page, SEL.saveSummary)`; return
    `page.locator(SEL.saveSummary)`.

- Replace inline `create_post` API copies: `posts.spec.ts` (5 sites:
  652/685/758/815/847), `authed-flash.spec.ts` local `createPublishedPostViaApi`
  (remove it), `feeds.spec.ts` local `publishPost` (remove it; update the
  `websub.ts:17` doc-comment reference).
- Adopt `composePost` at **≥1** `goto(page, "/posts/new")` composer site.
- Leave `visibility`'s `publishWithBaseAudience` (UI + audience) as-is unless it
  folds cleanly into `composePost` with an optional `audience` — either is fine
  so long as no duplicated create-post block remains.

**Verify (AC4):** `rg 'request\.post\([^)]*create_post' end2end/tests/*.spec.ts`
→ 0; `publishPost`/`createPublishedPostViaApi` gone;
`rg 'composePost\(' end2end/tests/*.spec.ts` non-empty;
`cargo xtask e2e-local posts.spec.ts` and `feeds.spec.ts` +
`authed-flash.spec.ts` green.

**Done when:** `[ ]` `posts.ts` created, API copies + local publish helpers
replaced, `composePost` adopted, affected specs green.

---

## Task 6 — `fillLoginForm` + login refactor + timed `click()` (#264)

**Files — `end2end/tests/helpers.ts`:**

- Add
  `export async function fillLoginForm(page: Page, username: string, password: string): Promise<void>`
  — fill `SEL.username`/`SEL.password`, `click(page, SEL.submit)`; **no
  navigation, no success wait**. Import `SEL`.
- Refactor `login(page, username, password, firstNavigationTimeoutMs?)`:
  **keep** its opening
  `goto(page, "/login", { timeout: firstNavigationTimeoutMs })`, then
  `fillLoginForm(...)`, then `waitForSelector(page, SEL.logoutLink)`. Signature
  and success signal unchanged.
- **Spec migration:** `auth.spec.ts` / `password_reset.spec.ts` error-path tests
  (the ~9 re-inlined fill+submit blocks) call `fillLoginForm(...)` then assert
  `SEL.error`. Route the raw `page.click('button[type="submit"]')` in
  `admin-site.spec.ts:29` and `email.spec.ts:17` through
  `click(page, SEL.submit)`.

**Verify (AC6):**
`rg -g '!mail.ts' "page\.click\('button\[type=\"submit\"\]'\)" end2end/tests`
returns 0 (the fixtures.ts one was handled in Task 3; `mail.ts:19` is a
doc-comment example, not code — excluded, not removed); `fillLoginForm` exported
and used by the error-path tests; `cargo xtask e2e-local` on `auth.spec.ts`,
`password_reset.spec.ts`, `admin-site.spec.ts`, `email.spec.ts` green.

**Done when:** `[ ]` `fillLoginForm` extracted, `login` rebuilt on it,
error-path + raw-click sites migrated, affected specs green.

---

## Task 7 — `extractToken` in `mail.ts` + collapse (#265)

**Files — `end2end/tests/mail.ts`:**

- Add `export function extractToken(email: CapturedEmail): string` —
  `const m = email.body_text.match(/token=([^\s]+)/); if (!m) throw new Error("no token in captured email"); return m[1];`
- Collapse the 3 parse sites to `const token = extractToken(mail)`:
  `verifiedUser` fixture (`fixtures.ts:283`), `email.spec.ts:25`,
  `password_reset.spec.ts:28` (dropping their
  `expect(tokenMatch).not.toBeNull()` soft-asserts).

**Verify (AC7):** `rg 'match\(/token=' end2end/tests` → 0; `extractToken` used
at the 3 sites; `cargo xtask e2e-local` on `email.spec.ts` +
`password_reset.spec.ts` green (verifiedUser exercised by any test using it).

**Done when:** `[ ]` `extractToken` added, 3 sites collapsed, affected specs
green.

---

## Task 8 — `expectFlash` (opportunistic, #266)

Attempt only if it falls out cleanly (~4 explicit-timeout sites).

- If cheap: add
  `export async function expectFlash(page: Page, text: string, timeout?: number)`
  wrapping `expect(page.locator(\`p:has-text("\${text}")\`)).toBeVisible({
  timeout })`(to`helpers.ts`), adopt at the `toBeVisible({ timeout
  })` confirmation sites (`fixtures.ts`, `admin-site`, `email`, `authed-flash`, `password_reset`).
- If it does **not** fall out naturally: leave `#266` open and note it in Task 9
  / the PR body (AC8 — must not be silently skipped).

**Verify:** if adopted, `cargo xtask e2e-local` on affected specs green.

**Done when:** `[ ]` `expectFlash` adopted, **or** #266 explicitly left open
with a recorded note.

---

## Task 9 — Full gate + acceptance sweep

- Run `cargo xtask validate` (all four `{sqlite,postgres}×{chromium,firefox}`
  combos) — must be green (AC9). Beware the stale-`:3000`-server false negative
  before trusting any pre-`validate` local run.
- Walk the spec's acceptance criteria 1–10 and confirm each grep/observation
  holds.
- Confirm the Task 1 follow-up issue exists and reference it (AC10).
- Hand off to **`jaunder-ship`** (final review, archive spec+plan, PR
  referencing #260 and closing #261–#266 as delivered, #266 noted if left open).

**Done when:** `[ ]` `cargo xtask validate` green, all ACs confirmed, ready for
ship.

# Spec — issue #260: reduce E2E test boilerplate (shared helpers + ambient timeouts)

**Issue:** [#260](https://github.com/jaunder-org/jaunder/issues/260) (tracking)
— sub-issues #261–#266. **Milestone:** E2E test suite. **Status:** design
resolved; awaiting approval.

## Goal

The Playwright e2e suite (`end2end/tests/*.spec.ts`, ~2400 lines across 13 spec
files) carries repetitious boilerplate: per-test timeout-scaling rituals,
inlined post-creation, literaled CSS selectors, re-inlined login fills,
duplicated email token parsing. Extract shared helpers and make timeout scaling
ambient so tests are more concise and consistent — **without changing what any
test verifies or (beyond the timeout-budget change spelled out below) how long
it is allowed to run**.

This is a refactor: the set of behaviours the suite exercises is unchanged. The
one intentional behavioural change is the timeout model in §1 (budgets only ever
rise, never fall).

## Non-goals

- **Right-sizing timeout budgets from measured durations.** Whether the
  surviving large budgets (60/90/150 s) are inflated is a separate, data-driven
  change that can _lower_ budgets and therefore risks flakes under worker
  contention. It is filed as a follow-up issue (the plan's first task) and is
  out of scope here.
- Splitting the large `fixtures.ts` (751 lines) or introducing a `helpers/`
  subdirectory. New helpers are flat topical siblings of the existing modules.
- Converting one-off / rarely-repeated selectors to named constants (§3 is the
  high-leverage set only).
- Changing the scaling factors, the worker-contention curve, or the
  `slowBrowser*` helpers' math.

## Design decisions (resolved in interview)

### 1. Ambient timeout scaling (#261)

Most heavy tests destructure `testInfo`, call
`test.setTimeout(slowBrowserTimeoutMs(testInfo, N))` (a few — `feeds`, `atompub`
— already use `const info = test.info(); info.setTimeout(...)`, the partial
precursor of this change), compute a first-nav budget with
`slowBrowserFirstNavigationTimeoutMs(testInfo, M)`, and thread it into
`register()`/`login()`/`goto()`. Whole-test budgets range 10 s–150 s across ~40
call sites; the Playwright config default is `timeout: 30 * 1000` **unscaled**.

Resolution — make scaling ambient, keep the per-test budget as intrinsic data:

- **Ambient default via an `auto` fixture.** A new `auto: true` fixture calls
  `testInfo.setTimeout(slowBrowserTimeoutMs(testInfo, DEFAULT_TEST_BUDGET_MS))`
  for **every** test. `DEFAULT_TEST_BUDGET_MS = 30_000` (named const in
  `fixtures.ts`). Playwright runs **all** auto fixtures before any
  test-requested fixture (`user`/`verifiedUser`/`registeredPage`), so this
  budget is in force before their setup runs — covering fixture setup + body +
  teardown with scaled headroom for every test. (Ordering _between_ the two auto
  fixtures — this one and the existing `_autoPerfSpan` — is not guaranteed by
  key position, but is immaterial: `_autoPerfSpan` setup is cheap and its warmup
  is env-gated off in CI.)
- **Retire `verifiedUser`'s hand-rolled line.** The ambient fixture makes
  `verifiedUser`'s existing
  `testInfo.setTimeout(slowBrowserTimeoutMs(testInfo, 30_000))`
  (fixtures.ts:271, same value) redundant — the auto fixture now provides
  exactly that coverage before `verifiedUser` runs. Remove that line and replace
  its load-bearing comment with a one-line pointer to the ambient fixture, so a
  future reader does not re-derive the setup-coverage concern.
- **`setTestBudget(ms)` for tests needing more than the default.** A new helper
  exported from `fixtures.ts`:
  ```ts
  export function setTestBudget(chromiumBudgetMs: number): void {
    const info = test.info();
    info.setTimeout(slowBrowserTimeoutMs(info, chromiumBudgetMs));
  }
  ```
  Called as the **first line of the test body, before any awaited body-level
  setup** (e.g. an in-body `register()`/`goto()`), so that setup runs under the
  raised budget rather than the ambient 30 s default. Reads `test.info()`
  internally, so the call site names neither `testInfo` nor the scaler function
  — only the intrinsic budget number remains. Used only by the tests whose
  budget exceeds `DEFAULT_TEST_BUDGET_MS` (the 60/90/150 s tests in `feeds`,
  `visibility`, `atompub`, `posts`).
- **Drop duplicative budget calls.** Every test whose current whole-test budget
  is **≤ `DEFAULT_TEST_BUDGET_MS`** drops its
  `test.setTimeout(slowBrowserTimeoutMs(...))` line entirely and inherits the
  ambient default. This _raises_ those tests' budgets (e.g. 10 s → scaled 30 s);
  a larger liveness budget can only delay the failure of a genuinely-hung test,
  never cause a new failure — so it is safe.
- **`firstNav` fixture value.** Expose the scaled first-navigation budget as a
  test-scoped fixture value —
  `slowBrowserFirstNavigationTimeoutMs(testInfo, 10_000)`, the **modal**
  test-body cold-WASM budget (the 23 `register(...)` sites in `posts`, plus
  `authed-flash`, `auth`, and `unicode-slug`, all pass `10_000`) — so those
  tests write `async ({ page, firstNav }) => …` instead of recomputing it.
  **Scope, and the no-lowering guarantee:** `firstNav` is fixed at the 10 000
  budget, and is adopted **only** at sites whose first-nav budget is _exactly_
  `10_000` today. Sites that pass a **different** first-nav budget must **not**
  be routed through `firstNav`, because that could _raise or lower_ a cold-WASM
  navigation timeout and a _lowering_ risks a flake under contention (the
  "budgets only ever rise" guarantee covers _whole-test_ budgets, not first-nav
  budgets). These keep their explicit
  `slowBrowserFirstNavigationTimeoutMs(testInfo, N)`: `posts.spec.ts:196`
  (**`12_000`** — split across lines 202-205; note this is a `posts` test that
  does **not** use the modal budget), `visibility` (`20_000`), and
  `feeds`/`atompub`/`media` (`30_000`). No first-nav budget is changed.
- **`registeredPage` fixture.** A `page` already registered with a fresh unique
  user using the `10_000`-based scaled `firstNav` budget, collapsing the
  `register(page, firstNav)` preamble in the tests that use the `10_000`
  first-nav budget **and discard the returned username**. Tests that _capture_
  the username (`posts` 290/338/345/395/402, `authed-flash:42`) or whose
  first-nav budget is not `10_000` keep their explicit `register(...)` call;
  tests that need the _credentials_ keep using the existing `user` /
  `verifiedUser` fixtures.
- **Rationale documentation** follows the existing convention — module doc
  comments in `fixtures.ts` (where the scaling rationale and #155 references
  already live), not a new ADR.

Verified fact grounding this design: whole-test `setTimeout` budgets in the
suite are `10_000, 12_000, 15_000, 20_000` (≤ default, will be dropped) and
`30_000, 60_000, 90_000, 150_000` (30 000 == default → dropped; > 30 000 keep an
explicit `setTestBudget`). The tests > 30 s are: `feeds` (60/150/90/60/60),
`visibility` (60/60/90/90), `atompub` (60/60/90), `posts` (60 at :680).

### 2. Shared post-creation helpers (#262) — new `posts.ts`

Promote the half-extracted `publishPost` (currently local to `feeds.spec.ts`)
and the ~6 inline `page.request.post(.../api/create_post)` copies into a shared
module:

```ts
createPostViaApi(page, opts: { body: string; tags?: string[]; publish?: boolean; slug?: string | null }): Promise<{ post_id: number; permalink: string }>
composePost(page, opts: { body: string; summary?: string; slug?: string; publish: boolean }): Promise<Locator /* .j-save-summary */>
```

- `createPostViaApi` posts to `${BASE_URL}/api/create_post` with the wire shape
  all current callers use — `format: "markdown"` hardcoded, `opts.slug` mapped
  to the `slug_override` field (default `null`), `opts.publish` **defaulting to
  `true`** (every current caller publishes). It asserts `res.ok()` **with a
  descriptive failure message built in** (fixing the context-free assertions in
  `posts.spec.ts`) and returns typed parsed JSON.
- `composePost` performs the **`/posts/new` UI-composer** flow
  (`goto(/posts/new)` → fill `SEL.postBody`, and, when provided, the
  summary/slug inputs → `click(SEL.publishButton(value))` →
  `waitForSelector(SEL.saveSummary)`) and returns the save-summary locator.
  `publish` is required. **Scope:** it serves the `goto(page, "/posts/new")`
  composer sites only; the home-page `.j-composer` compose flow (`posts.spec.ts`
  ~486-585, which does _not_ navigate to `/posts/new`) is a different path and
  is out of scope for `composePost`.
- Replace the inline API copies at `posts.spec.ts` (5), `authed-flash.spec.ts`
  (its local `createPublishedPostViaApi`), and `feeds.spec.ts`'s local
  `publishPost`. `visibility.spec.ts`'s `publishWithBaseAudience` is a
  _UI-composer_ variant (it navigates `/posts/new`,
  `selectOption("#audience-base", …)`, and reads a permalink), **not** an API
  call — so if consolidated it belongs with `composePost` (e.g. an optional
  `audience` option), never with `createPostViaApi`. Leaving it local is
  acceptable so long as no _duplicated_ create-post block remains; the plan
  decides.

### 3. Named selector constants (#263) — new `selectors.ts`, high-leverage set

Export a `SEL` object covering the high-frequency selectors, giving a single
source of truth (a markup rename touches one place) and normalising quote style.
(Note: the suite is already internally consistent _per selector_ —
`a[href='/logout']` is uniformly single-quoted, `button[type="submit"]`
uniformly double-quoted — so this is a cross-selector style unification and
single-source-of-truth win, not a fix for mixed quoting of the same selector.)

`SEL.saveSummary` (`.j-save-summary`), `SEL.postBody` (`textarea[name="body"]`),
`SEL.publishButton(value)` (function → `button[name="publish"][value="…"]`),
`SEL.error` (`.error`), `SEL.submit` (`button[type="submit"]`), `SEL.logoutLink`
(`a[href="/logout"]`), `SEL.username` (`input[name="username"]`), `SEL.password`
(`input[name="password"]`), `SEL.topbarHeading` (`.j-topbar h1`).

One-off selectors (e.g. `input[name="email"]`, `input[name="slug_override"]`)
stay inline.

### 4. `fillLoginForm` + consistent timed `click()` (#264)

- Extract `fillLoginForm(page, username, password)` into `helpers.ts` — fills
  `SEL.username`/`SEL.password` and submits via `click(page, SEL.submit)`, **no
  navigation and no success wait**.
- `login(page, username, password, firstNavigationTimeoutMs?)` **keeps its
  opening `goto(page, "/login", { timeout: firstNavigationTimeoutMs })`** (the
  cold-nav budget, unchanged — `fillLoginForm` does not navigate), then becomes
  `fillLoginForm(...)` + `waitForSelector(page, SEL.logoutLink)` (its existing
  success signal, unchanged). Its signature is unchanged.
- Error-path tests (`auth.spec.ts`, `password_reset.spec.ts`) call
  `fillLoginForm(...)` then assert on `SEL.error`, replacing the re-inlined
  fill+submit blocks.
- Route the raw `page.click('button[type="submit"]')` in `admin-site.spec.ts`,
  `email.spec.ts`, and the `verifiedUser` fixture through the timed `click()`
  wrapper so those interactions appear in the OTEL trace. (`fixtures.ts`
  currently imports only `{ goto, login, register }` from `./helpers` — add
  `click` to that import.)

### 5. `extractToken` email helper (#265)

Add to `mail.ts`:

```ts
export function extractToken(email: CapturedEmail): string;
```

Matches `/token=([^\s]+)/` in `email.body_text`; **throws** on miss. Collapse
the three duplicated parse sites (`verifiedUser` fixture, `email.spec.ts`,
`password_reset.spec.ts`) to `const token = extractToken(mail)`. The
`verifiedUser` fixture already throws; `email.spec.ts` and
`password_reset.spec.ts` currently use `expect(tokenMatch).not.toBeNull()` —
converting them to the throwing helper still fails the test on a missing token,
but as a thrown error rather than a soft assertion (an acceptable, minor change,
not strictly behaviour-preserving).

### 6. `expectFlash` confirmation waits (#266) — opportunistic, low priority

If it falls out naturally, add `expectFlash(page, text, timeout?)` wrapping the
`expect(page.locator('p:has-text("…")')).toBeVisible({ timeout })` idiom and
standardising the ad-hoc timeout. Only ~4 sites use the explicit-timeout form,
so this is include-if-cheap; if it does not fall out naturally, #266 stays open
and is noted as such at ship.

## File layout

Flat topical siblings, matching the existing convention (`mail.ts`,
`hydration.ts`, `actions.ts`, `seed.ts`, `websub.ts`):

| File                         | Change                                                                                                                |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `end2end/tests/fixtures.ts`  | + auto-timeout fixture, `DEFAULT_TEST_BUDGET_MS`, `setTestBudget`, `firstNav` fixture value, `registeredPage` fixture |
| `end2end/tests/selectors.ts` | **new** — `SEL` constants                                                                                             |
| `end2end/tests/posts.ts`     | **new** — `createPostViaApi`, `composePost`                                                                           |
| `end2end/tests/helpers.ts`   | + `fillLoginForm`; `login` refactored on top of it                                                                    |
| `end2end/tests/mail.ts`      | + `extractToken`                                                                                                      |
| `end2end/tests/*.spec.ts`    | call sites migrated to the above                                                                                      |

## Acceptance criteria (observable)

1. **Ambient timeout.** The whole-test `setTimeout` ritual is gone from the
   specs: `rg 'setTimeout\(slowBrowserTimeoutMs' end2end/tests/*.spec.ts`
   returns **0** (the scaling call now lives only in `fixtures.ts`), and no spec
   destructures `testInfo` _solely_ to set a whole-test timeout. The scaling for
   `> 30 000` tests is expressed as `setTestBudget(N)`:
   `rg 'setTestBudget\(' end2end/tests/*.spec.ts` returns exactly the 13
   enumerated `> 30 000` sites and no others (all with `N > 30_000`).
2. **`setTestBudget`** exists in `fixtures.ts`, applies the same scaling as
   `slowBrowserTimeoutMs`, and is used by exactly the > 30 000 tests enumerated
   in §1 (feeds ×5, visibility ×4, atompub ×3, posts ×1 = 13).
3. **`registeredPage` / `firstNav` fixtures** exist. `firstNav` equals
   `slowBrowserFirstNavigationTimeoutMs(testInfo, 10_000)`. The `10_000`
   first-nav sites (the modal `posts` register sites, `authed-flash`, `auth`,
   `unicode-slug`) consume `firstNav` (and, where they open with a bare register
   whose username they discard, `registeredPage`) instead of recomputing the
   budget; the non-`10_000` first-nav sites — `posts.spec.ts:196` (`12_000`),
   `visibility` (`20_000`), `feeds`/`atompub`/`media` (`30_000`) — still carry
   an explicit `slowBrowserFirstNavigationTimeoutMs(testInfo, N)`. No first-nav
   budget is changed:
   `rg 'slowBrowserFirstNavigationTimeoutMs\(testInfo, 10_000\)' end2end/tests/*.spec.ts`
   returns 0 (all modal sites migrated).
4. **Post helpers.** `end2end/tests/posts.ts` exports `createPostViaApi` and
   `composePost` with the signatures in §2. No `page.request.post` targeting
   `/api/create_post` remains inline in any spec; the local `publishPost`
   (`feeds`) and `createPublishedPostViaApi` (`authed-flash`) are removed in
   favour of the shared helper. `createPostViaApi` asserts `res.ok()` with a
   descriptive message, hardcodes `format: "markdown"`, maps
   `slug → slug_override`, and defaults `publish` to `true`. **`composePost` is
   adopted** at ≥1 of the `goto(page, "/posts/new")` composer sites (it is not
   exported-but-unused); `rg 'composePost\(' end2end/tests/*.spec.ts` is
   non-empty.
5. **Selectors.** `end2end/tests/selectors.ts` exports the `SEL` members in §3.
   No **standalone** literal of those selectors remains in `*.spec.ts` (all
   routed through `SEL`), and no single-vs-double-quote variants of the
   logout/submit selectors remain. **Exempt:** compound selectors that _embed_ a
   `SEL` string (e.g. `'.j-app-passwords button[type="submit"]'` in
   `atompub.spec.ts`) and the one-off selectors §3 explicitly leaves inline — an
   `rg` for the bare string may still match those, which is expected.
6. **`fillLoginForm`.** `helpers.ts` exports `fillLoginForm`; `login` is
   implemented on top of it. `auth.spec.ts`/`password_reset.spec.ts` error-path
   tests use `fillLoginForm`. No raw `page.click(` for a submit button remains
   in `admin-site.spec.ts`, `email.spec.ts`, or the `verifiedUser` fixture (all
   go through `click()`).
7. **`extractToken`.** `mail.ts` exports `extractToken`; the three former parse
   sites call it; it throws on a missing token.
8. **`expectFlash`** — either implemented and adopted at the confirmation-wait
   sites, or #266 explicitly left open with a one-line note; not silently
   skipped.
9. **Behaviour unchanged / green gate.** `cargo xtask validate` passes (static +
   coverage + e2e across all four `{sqlite,postgres}×{chromium,firefox}`
   combos). No test's set of assertions changed; the only timeout change is
   budgets rising to the ambient default for the ≤ 30 000 tests.
10. **Follow-up filed.** A GitHub issue "right-size e2e timeout budgets from
    measured `results.json` durations" exists and is linked from this cycle's PR
    / the plan's first task.

## Constraints / invariants preserved

- The `goto`/`click`/`waitForSelector`/`click`-through-OTEL convention
  (`helpers.ts` module doc) — new helpers use the wrappers, never raw
  `page.goto`/`page.click`.
- "Never `waitForLoadState("networkidle")`; wait for a specific element."
- "Prefer `waitForNewEmail(previousCount)`; snapshot the count before the
  action."
- The recipient-scoped `mailbox` fixture semantics.
- Backend parity and coverage policy (CONTRIBUTING.md); the verify ladder.

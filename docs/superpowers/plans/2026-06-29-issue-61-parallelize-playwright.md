# Parallelize the Playwright e2e suite (#61) — Implementation Plan

> **OUTCOME (2026-06-30): partially executed.** Tasks 1–5 + 7 (the per-test
> identity fixtures, spec migrations, own-scoped feed assertion, and ADR) landed
> as **parallel-safe prep**. Task 6 (the `workers > 1` flip + serial project + VM
> bump) and Task 8 (validation) were **reverted/deferred**: enabling concurrent
> SSR exposed reactive-disposal panics that no available fix resolves — tracked as
> **#173**, which now blocks #61. The gate stays `workers: 1`. See ADR-0039.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the Nix-VM Playwright e2e suite with `workers > 1` by making every spec parallel-safe through per-test identity fixtures, with the one global-singleton spec (`admin-site`) quarantined in a serial Playwright project.

**Architecture:** Add three lazy, test-scoped fixtures to the existing `end2end/tests/fixtures.ts` — `user` (out-of-band unique account), `mailbox` (recipient-scoped mail waiter), `verifiedUser` (out-of-band verified account). Migrate the four specs that still depend on shared seeded accounts / global feed counts to those fixtures. Restructure `nixPlaywrightConfig` into per-browser **parallel** projects (`fullyParallel`, `workers: 4`, exclude `admin-site`) plus per-browser **serial** projects (`admin-site` only, `dependencies` on the parallel project so they run in a non-overlapping phase). Raise VM capacity to 4 cores / 4 GB.

**Tech Stack:** Playwright 1.58 (TypeScript), Nix flake `nixosTest` VMs, `cargo xtask validate`.

## Global Constraints

- **No Co-Authored-By trailers** in any commit (repo policy).
- **Per-commit gate is git-enforced** (pre-commit: `cargo xtask check --no-test` + `validate --no-e2e --allow-dirty`). Run it from inside the worktree.
- **Scope boundary:** change **only** `nixPlaywrightConfig` (the gate-relevant config) and the spec/fixture files. Do **not** touch the local `end2end/playwright.config.ts` — deduping the two configs is **#153** (out of scope).
- **`register()` password is the literal `"testpassword123"`** (hardcoded in `helpers.ts:132`). Any `user.password` must equal it.
- **`register()` returns only a username**; an account has no email until one is set via `/profile/email`. The fixture's `email` is the deterministic, unique `` `${username}@example.com` ``.
- **`admin-site` keeps the seeded `testoperator`/`testlogin` accounts** — operator privilege cannot be self-served via `register()`. It is parallel-unsafe because it mutates the singleton `site.title`/`base_url`, so it is quarantined by config, not de-seeded.
- **The behavioral gate is the Nix-VM run** (Task 8), per the spec's flaky-shaped validation strategy. Per-spec edits cannot be cheaply run in isolation; their per-task gate is prettier + `tsc --noEmit`, and behavioral correctness is proven in Task 8 (Postgres-first → SQLite → repeats → full `validate`).

---

### Task 1: Identity fixtures — `user`, `mailbox`, `verifiedUser`

**Files:**

- Modify: `end2end/tests/fixtures.ts` (add imports, types, and three fixtures to the existing `base.extend<...>` call at lines ~190-741; re-export unchanged)

**Interfaces:**

- Consumes: `register(page, firstNavTimeoutMs): Promise<string>`, `login(page, username, password, firstNavTimeoutMs?)`, `goto(page, path, opts?)` from `./helpers`; `readEmailLines(): string[]`, `CapturedEmail { to: string[]; from: string|null; subject: string; body_text: string }` from `./mail`; `hydrationHeavyFirstNavigationTimeoutMs(testInfo, ms)` (already defined in this file).
- Produces:
  - `type TestUser = { username: string; password: string; email: string }`
  - `type Mailbox = { waitForNewEmail(timeoutMs?: number): Promise<CapturedEmail> }`
  - Fixtures `user: TestUser`, `mailbox: Mailbox`, `verifiedUser: TestUser` on the exported `test`.

**Why out-of-band:** `register()` auto-logs-in the page it runs on. Tests that exercise the **login form** or **password reset** need an account they are **not** currently logged into, so `user`/`verifiedUser` provision the account in a throwaway `browser` context and leave the test's own `page` untouched (logged out).

- [x] **Step 1: Add the helper imports**

At the top of `fixtures.ts`, the existing import block pulls `expect`, `test as base`, and types from `@playwright/test`. Add two import lines after the `./otel` import (around line 23):

```ts
import { goto, login, register } from "./helpers";
import { readEmailLines, type CapturedEmail } from "./mail";
```

(No circular import: `helpers.ts` imports only `./actions` + `./hydration`; `mail.ts` imports only `fs`. Neither imports `./fixtures`.)

- [x] **Step 2: Add the exported types**

Just above the `const test = base.extend<...>` declaration (around line 190), add:

```ts
/** A uniquely-named account provisioned per test. `password` is the literal
 *  `register()` password; `email` is the deterministic unique address this
 *  account uses when it sets/verifies email. */
export type TestUser = { username: string; password: string; email: string };

/** A recipient-scoped mail waiter bound to one `TestUser.email`. Each call
 *  returns that recipient's next unseen message (FIFO), so parallel tests
 *  never consume each other's mail. */
export type Mailbox = {
  waitForNewEmail(timeoutMs?: number): Promise<CapturedEmail>;
};
```

- [x] **Step 3: Widen the `base.extend` generic and add the three fixtures**

Change the generic on the existing extend call from `base.extend<{ _autoPerfSpan: void }>(` to:

```ts
const test = base.extend<{
  _autoPerfSpan: void;
  user: TestUser;
  mailbox: Mailbox;
  verifiedUser: TestUser;
}>({
```

Then, **inside** the `extend({ ... })` object literal (alongside `_autoPerfSpan`), add the three fixtures. Place them before the closing `}` of the object (after the `_autoPerfSpan` entry):

```ts
  // A uniquely-named account, registered in a throwaway context so the test's
  // own `page` stays logged out. Lazy: only provisioned for tests that
  // destructure `user`.
  user: async ({ browser }, use, testInfo) => {
    const context = await browser.newContext();
    const page = await context.newPage();
    const username = await register(
      page,
      hydrationHeavyFirstNavigationTimeoutMs(testInfo, 15_000),
    );
    await context.close();
    await use({
      username,
      password: "testpassword123",
      email: `${username}@example.com`,
    });
  },

  // Recipient-scoped mail waiter. Filters mail.jsonl by `user.email` and tracks
  // a per-mailbox cursor so each call returns this recipient's next message.
  mailbox: async ({ user }, use) => {
    const matching = () =>
      readEmailLines()
        .map((line) => JSON.parse(line) as CapturedEmail)
        .filter((mail) => mail.to.includes(user.email));
    // Seed the cursor at any pre-existing matching mail (there should be none,
    // since the address is unique to this test).
    let cursor = matching().length;
    const waitForNewEmail = async (
      timeoutMs = 5_000,
    ): Promise<CapturedEmail> => {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        const mails = matching();
        if (mails.length > cursor) {
          const next = mails[cursor];
          cursor += 1;
          return next;
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      throw new Error(`timed out waiting for email to ${user.email}`);
    };
    await use({ waitForNewEmail });
  },

  // `user` plus the email set-and-verify flow, driven through `mailbox`, all
  // out-of-band so the test's `page` stays logged out. Yields the same
  // credentials; the account now has a verified email.
  verifiedUser: async ({ browser, user, mailbox }, use, testInfo) => {
    const context = await browser.newContext();
    const page = await context.newPage();
    const firstNav = hydrationHeavyFirstNavigationTimeoutMs(testInfo, 15_000);
    await login(page, user.username, user.password, firstNav);
    await goto(page, "/profile/email");
    await page.fill('input[name="email"]', user.email);
    await page.click('button[type="submit"]');
    await expect(
      page.locator('p:has-text("Check your email")'),
    ).toBeVisible({ timeout: 10_000 });
    const mail = await mailbox.waitForNewEmail();
    const tokenMatch = mail.body_text.match(/token=([^\s]+)/);
    if (!tokenMatch) throw new Error("no verification token in captured email");
    await goto(page, `/verify-email?token=${tokenMatch[1]}`);
    await expect(page.locator('p:has-text("verified")')).toBeVisible();
    await context.close();
    await use(user);
  },
```

(The trailing re-export `export { expect, test };` at the end of the file stays unchanged.)

- [x] **Step 4: Format and typecheck**

> **Gate note (decided during execution):** `tsc --noEmit` is **not** the project gate — nothing in CI/the commit hook runs a type-checker, and HEAD already has 14 latent `tsc` errors in pre-existing `_autoPerfSpan` code. The real gate is **prettier** (enforced by the commit hook). Per-task gate going forward is **prettier only**; the tsc gap is filed as **#169**. My fixtures add **zero** new tsc errors (14 → 14).

```bash
prettier -w end2end/tests/fixtures.ts
```

Expected: prettier rewrites cleanly; `tsc --noEmit` exits 0 (no type errors). `npm ci` is a one-time install of the `end2end` dev deps (Playwright + typescript); it is needed for `tsc` and is cached for later tasks.

- [x] **Step 5: Commit**

```bash
git add end2end/tests/fixtures.ts
git commit -m "test(e2e): add per-test identity fixtures (user/mailbox/verifiedUser) (#61)"
```

(Pre-commit hook runs `cargo xtask check --no-test` — which prettier-checks `end2end` — plus `validate --no-e2e --allow-dirty`. Both must pass.)

---

### Task 2: Migrate `email.spec.ts` to `user` + `mailbox`

**Files:**

- Modify: `end2end/tests/email.spec.ts:6-40` (the "email verification flow completes successfully" test)

**Interfaces:**

- Consumes: `user`, `mailbox` fixtures (Task 1).

The second test ("invalid token shows error", lines 42-49) needs **no change** — it touches no account or mail.

- [x] **Step 1: Rewrite the verification-flow test to self-provision its user**

Replace lines 1-40 (imports + first test) with:

```ts
import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { goto, login } from "./helpers";

// M3.10.11: Full email verification flow.
test("email verification flow completes successfully", async ({
  page,
  user,
  mailbox,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));

  await login(page, user.username, user.password);

  // Navigate to email settings and submit this user's unique address.
  await goto(page, "/profile/email");
  await page.fill('input[name="email"]', user.email);
  await page.click('button[type="submit"]');

  await expect(page.locator('p:has-text("Check your email")')).toBeVisible({
    timeout: 10_000,
  });

  // Read this recipient's verification mail (recipient-scoped, parallel-safe).
  const email = await mailbox.waitForNewEmail();
  const tokenMatch = email.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the verification link.
  await goto(page, `/verify-email?token=${token}`);
  await expect(page.locator('p:has-text("verified")')).toBeVisible();

  // Confirm email is shown as verified on the profile page.
  await goto(page, "/profile/email");
  await expect(page.locator("p")).toContainText("verified");
});
```

Note: the old `import { readEmailLines, waitForNewEmail } from "./mail";` line is dropped (the mailbox fixture replaces it). The `login`/`goto` import stays; `readEmailLines`/`waitForNewEmail` are no longer referenced in this file.

- [x] **Step 2: Format and typecheck**

```bash
prettier -w end2end/tests/email.spec.ts
( cd end2end && npx tsc --noEmit )
```

Expected: clean; exit 0.

- [x] **Step 3: Commit**

```bash
git add end2end/tests/email.spec.ts
git commit -m "test(e2e): self-provision the email-verification user via fixtures (#61)"
```

---

### Task 3: Migrate `password_reset.spec.ts` to `verifiedUser`/`user` + `mailbox`

**Files:**

- Modify: `end2end/tests/password_reset.spec.ts` (tests at lines 6-56 and 71-81)

**Interfaces:**

- Consumes: `verifiedUser`, `user`, `mailbox` fixtures (Task 1).

The "invalid token" test (lines 58-68) needs **no change**.

- [x] **Step 1: Rewrite the imports**

Replace line 3 (`import { readEmailLines, waitForNewEmail } from "./mail";`) — delete it. Keep line 1-2:

```ts
import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { goto, click, waitForSelector, waitForHydration } from "./helpers";
```

- [x] **Step 2: Rewrite the "password reset flow completes successfully" test**

Replace lines 6-56 with a version that uses a self-provisioned verified user and reads its own reset mail. The old password is `verifiedUser.password` (`"testpassword123"`); the new password stays `"resetpassword789"`:

```ts
// M3.11.13: Full password reset flow.
test("password reset flow completes successfully", async ({
  page,
  verifiedUser,
  mailbox,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));

  // Request a password reset for this test's own verified user.
  await goto(page, "/forgot-password");
  await page.fill('input[name="username"]', verifiedUser.username);
  await click(page, 'button[type="submit"]');

  // Page should show a neutral confirmation (not confirm whether user exists).
  await expect(page.locator("p")).toContainText(/check|sent|email/i, {
    timeout: 10_000,
  });

  // Read this recipient's reset mail (recipient-scoped, parallel-safe).
  const email = await mailbox.waitForNewEmail();
  const tokenMatch = email.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the reset link and submit a new password.
  await goto(page, `/reset-password?token=${token}`);
  await page.fill('input[name="new_password"]', "resetpassword789");
  await click(page, 'button[type="submit"]');
  // Wait for the router redirect to /login — ensures the password change has
  // persisted before testing the old credential below.
  await page.waitForURL("**/login");

  // Login with the OLD password should fail.
  await goto(page, "/login");
  await page.fill('input[name="username"]', verifiedUser.username);
  await page.fill('input[name="password"]', verifiedUser.password);
  await click(page, 'button[type="submit"]');
  await expect(page.locator(".error")).toBeVisible({ timeout: 10_000 });

  // Login with the NEW password should succeed from the same hydrated page.
  await page.fill('input[name="username"]', "");
  await page.fill('input[name="password"]', "");
  await page.fill('input[name="username"]', verifiedUser.username);
  await page.fill('input[name="password"]', "resetpassword789");
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, "a[href='/logout']", { timeout: 10_000 });
  await waitForHydration(page);
  await expect(page.locator(".j-topbar h1")).toHaveText("Home");
});
```

- [x] **Step 3: Rewrite the "no verified email" test to use a fresh unverified `user`**

Replace lines 71-81 (the `testnoemail` test). A freshly-registered `user` has no verified email by definition, so it exercises the same "contact operator" path:

```ts
// M3.11.15: /forgot-password for a user with no verified email shows the
// "contact operator" error.
test("forgot-password for user without verified email shows contact operator error", async ({
  page,
  user,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await goto(page, "/forgot-password");
  // A freshly-registered user exists but has no verified email.
  await page.fill('input[name="username"]', user.username);
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, ".error");
  await expect(page.locator(".error")).toBeVisible();
});
```

- [x] **Step 4: Format and typecheck**

```bash
prettier -w end2end/tests/password_reset.spec.ts
( cd end2end && npx tsc --noEmit )
```

Expected: clean; exit 0.

- [x] **Step 5: Commit**

```bash
git add end2end/tests/password_reset.spec.ts
git commit -m "test(e2e): self-provision password-reset users via fixtures (#61)"
```

---

### Task 4: Migrate `auth.spec.ts` seeded-account usages to `user`

**Files:**

- Modify: `end2end/tests/auth.spec.ts` (tests at lines 49-71, 85-99, 101-117)

**Interfaces:**

- Consumes: `user` fixture (Task 1), `login` from `./helpers`.

Three tests reference `testlogin`. The other tests (register-form, wrong-password, signed-out sidebar, `register()`-based footer) are already unique-per-test and need **no change**.

- [x] **Step 1: Rewrite "login with valid credentials succeeds" (lines 49-71)**

This test exercises the **login form**, so it needs a registered-but-logged-out account — exactly `user`. Replace lines 49-71 with:

```ts
test("login with valid credentials succeeds", async ({
  page,
  user,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  const perf = createPerfProbe(testInfo, "auth_login_success");

  await goto(page, "/login");

  await page.fill('input[name="username"]', user.username);
  await page.fill('input[name="password"]', user.password);
  perf.mark("credentials_filled");
  await click(page, 'button[type="submit"]');
  perf.mark("submit_clicked");
  // waitForURL is unreliable in Firefox for location.replace() navigations; wait
  // for the sidebar logout link, which only appears after the Suspense resolves
  // with the authenticated state — by that point the navigation is fully settled.
  await waitForSelector(page, "a[href='/logout']");
  perf.mark("logout_link_visible");
  await waitForHydration(page);

  await expect(page.locator(".j-sb-foot")).toContainText(user.username);
  await expect(page.locator(".j-sidebar")).toBeVisible();
  perf.mark("assertions_complete");
  await perf.log();
});
```

- [x] **Step 2: Rewrite "logout page logs out" (lines 85-99)**

Replace lines 85-99 with a version that logs in as `user` then logs out:

```ts
test("logout page logs out", async ({ page, user }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await login(page, user.username, user.password);

  // Use the rendered logout link to avoid Firefox navigation abort races.
  await click(page, "a[href='/logout']");

  // Logout clears the session and redirects to "/"; waitForURL is reliable here
  // because logout is a server-side 302 redirect (not location.replace).
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await waitForHydration(page);
  // Footer shows neither username nor sign-in link after logout.
  await expect(page.locator(".j-sb-foot")).not.toContainText(user.username);
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});
```

- [x] **Step 3: Rewrite "sidebar reverts to signed-out state after logout" (lines 101-117)**

Replace lines 101-117 with:

```ts
test("sidebar reverts to signed-out state after logout", async ({
  page,
  user,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  await login(page, user.username, user.password);
  // a[href='/logout'] only renders when auth Suspense resolves, confirming the
  // user is shown.
  await expect(page.locator(".j-sb-foot")).toContainText(user.username);

  // Click the sidebar "Sign out" link and confirm the sidebar switches back.
  await click(page, "a[href='/logout']");
  // Logout is a server-side 302 redirect (not location.replace), so waitForURL is reliable.
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await waitForHydration(page);
  await expect(page.locator(".j-sb-foot")).not.toContainText(user.username);
  // Footer no longer shows a Sign-in link — it renders nothing when unauthenticated.
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});
```

- [x] **Step 4: Format and typecheck**

```bash
prettier -w end2end/tests/auth.spec.ts
( cd end2end && npx tsc --noEmit )
```

Expected: clean; exit 0. (`login`, `BASE_URL`, `createPerfProbe` are already imported in this file.)

- [x] **Step 5: Commit**

```bash
git add end2end/tests/auth.spec.ts
git commit -m "test(e2e): replace seeded testlogin with per-test user in auth specs (#61)"
```

---

### Task 5: Own-scope the `posts.spec.ts` guest local-timeline assertion

**Files:**

- Modify: `end2end/tests/posts.spec.ts:349-397` (the "home page shows local timeline for unauthenticated users" test)

**Interfaces:**

- Consumes: existing `register`, `createPublishedPostViaApi`, `perf` helpers in this file.

**Why only this one test:** every other `posts.spec` test already mints a unique user via `register()` and asserts against its own posts (the authenticated home-feed test at lines 399-448 is already own-scoped — the feed shows only the user's own posts). Only the **guest/anonymous local timeline** reads the global feed and asserts an exact `toHaveCount(TIMELINE_PAGE_SIZE)`, which a concurrent publisher can perturb. The `.j-topbar h1 === "jaunder.local"` title assertions (here and `example.spec.ts:9`) stay valid because `admin-site` — the only title mutator — runs in the serial project (Task 6) and never overlaps.

- [x] **Step 1: Replace the exact-count assertions with own-scoped / lower-bound checks**

In the test at lines 349-397, replace the assertion block (current lines 380-391):

```ts
await expect(guestPage.locator(".j-topbar h1")).toHaveText("jaunder.local");
await expect(guestPage.locator("article.j-post")).toHaveCount(
  TIMELINE_PAGE_SIZE,
);

await click(guestPage, 'button:has-text("Load more")');
perf.mark("load_more_clicked");
await expect
  .poll(async () => guestPage.locator("article.j-post").count(), {
    timeout: 10_000,
  })
  .toBeGreaterThan(TIMELINE_PAGE_SIZE);
```

with:

```ts
// Site title is still the seeded value: admin-site (the only mutator) runs in
// the serial Playwright project and never overlaps this test.
await expect(guestPage.locator(".j-topbar h1")).toHaveText("jaunder.local");

// Own-scoped: with workers>1 other tests publish into the same global local
// timeline, so assert a full first page exists rather than an exact count.
// This test alone seeds 2 * LOCAL_TIMELINE_AUTHOR_COUNT (52) posts, so a full
// page is guaranteed regardless of concurrent publishers.
await expect
  .poll(async () => guestPage.locator("article.j-post").count(), {
    timeout: 10_000,
  })
  .toBeGreaterThanOrEqual(TIMELINE_PAGE_SIZE);
const firstPageCount = await guestPage.locator("article.j-post").count();

// Pagination works: "Load more" grows the rendered set.
await click(guestPage, 'button:has-text("Load more")');
perf.mark("load_more_clicked");
await expect
  .poll(async () => guestPage.locator("article.j-post").count(), {
    timeout: 10_000,
  })
  .toBeGreaterThan(firstPageCount);
```

(The exact `toHaveCount(TIMELINE_PAGE_SIZE)` becomes `toBeGreaterThanOrEqual` per the spec; the load-more check now compares against the actual first-page count instead of the constant, so it stays correct even if the server returns a partial first page under load.)

- [x] **Step 2: Format and typecheck**

```bash
prettier -w end2end/tests/posts.spec.ts
( cd end2end && npx tsc --noEmit )
```

Expected: clean; exit 0.

- [x] **Step 3: Commit**

```bash
git add end2end/tests/posts.spec.ts
git commit -m "test(e2e): own-scope the guest local-timeline assertion for parallelism (#61)"
```

---

### Task 6: Gate config — parallel + serial projects, VM capacity

**Files:**

- Modify: `flake.nix:430-479` (`nixPlaywrightConfig`: comment, `workers`, `projects`)
- Modify: `flake.nix:586` (`e2eRunAndCapture`: add the serial project to the Playwright invocation)
- Modify: `flake.nix:649` (`mkE2eSqliteCheck` `virtualisation` block: add cores, raise memory)
- Modify: `flake.nix:739` (`mkE2ePostgresCheck` `virtualisation` block: add cores, raise memory)

**Interfaces:**

- Consumes: `admin-site.spec.ts` (routed by `testMatch`/`testIgnore`, no spec edit).
- Produces: per-browser project pair `${browser}` (parallel) + `${browser}-serial` (admin-site only).

- [x] **Step 1: Rewrite the `nixPlaywrightConfig` comment + `workers` + `projects`**

Replace the block from line 453 (`// Run spec files sequentially...`) through the end of the `projects: [...]` array (line 477) with:

```js
            // Run with multiple workers: the suite is parallel-safe via per-test
            // identity fixtures (unique accounts + recipient-scoped mailboxes,
            // see end2end/tests/fixtures.ts). The lone global-singleton spec,
            // admin-site, mutates site.title/base_url and is quarantined in a
            // per-browser serial project that runs in its own phase (project
            // `dependencies`) so it never overlaps the title-asserting specs.
            // SQLite write contention is handled by BEGIN IMMEDIATE (#51/#52/#53).
            workers: 4,
            projects: [
              {
                name: 'chromium',
                testIgnore: /admin-site\.spec\.ts/,
                fullyParallel: true,
                use: {
                  ...devices['Desktop Chrome'],
                  launchOptions: {
                    args: [
                      '--no-sandbox',
                      '--disable-gpu',
                      '--disable-dev-shm-usage',
                    ],
                  },
                },
              },
              {
                name: 'chromium-serial',
                testMatch: /admin-site\.spec\.ts/,
                fullyParallel: false,
                dependencies: ['chromium'],
                use: {
                  ...devices['Desktop Chrome'],
                  launchOptions: {
                    args: [
                      '--no-sandbox',
                      '--disable-gpu',
                      '--disable-dev-shm-usage',
                    ],
                  },
                },
              },
              {
                name: 'firefox',
                testIgnore: /admin-site\.spec\.ts/,
                fullyParallel: true,
                use: {
                  ...devices['Desktop Firefox'],
                },
              },
              {
                name: 'firefox-serial',
                testMatch: /admin-site\.spec\.ts/,
                fullyParallel: false,
                dependencies: ['firefox'],
                use: {
                  ...devices['Desktop Firefox'],
                },
              },
            ],
```

(`workers: 4` is global; `fullyParallel` is set per-project — `true` on the parallel projects, `false` on the serial ones so `admin-site`'s two tests run sequentially within their single file.)

- [x] **Step 2: Pass both the parallel and serial project to the VM Playwright run**

In `e2eRunAndCapture` (line 586), change:

```nix
              + " --config playwright.nix.config.js --project ${browser}"
```

to:

```nix
              + " --config playwright.nix.config.js --project ${browser} --project ${browser}-serial"
```

(With both selected, Playwright runs the parallel `${browser}` project across 4 workers, then — because `${browser}-serial` `dependencies` on `${browser}` — runs `admin-site` in a non-overlapping serial phase.)

- [x] **Step 3: Raise SQLite-VM capacity**

In `mkE2eSqliteCheck` (line 649), replace:

```nix
                virtualisation.memorySize = 2048;
```

with:

```nix
                # 4 cores + 4 GB so Playwright's 4 workers each get a vCPU and
                # 4 concurrent browser contexts don't thrash 2 GB (#61).
                virtualisation.cores = 4;
                virtualisation.memorySize = 4096;
```

- [x] **Step 4: Raise Postgres-VM capacity**

In `mkE2ePostgresCheck` (line 739), make the identical replacement:

```nix
                # 4 cores + 4 GB so Playwright's 4 workers each get a vCPU and
                # 4 concurrent browser contexts don't thrash 2 GB (#61).
                virtualisation.cores = 4;
                virtualisation.memorySize = 4096;
```

- [x] **Step 5: Validate the flake evaluates**

```bash
nix flake check --no-build 2>&1 | tail -n 20 || true
nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.drvPath
```

Expected: the flake parses (no Nix syntax/eval errors); the `nix eval` prints a `/nix/store/...drv` path (the changed config + VM settings produce a valid derivation). A non-empty drvPath confirms evaluation succeeded.

- [x] **Step 6: Commit**

```bash
git add flake.nix
git commit -m "ci(e2e): run Playwright with workers=4 + serial admin-site project; 4-core/4GB VMs (#61)"
```

---

### Task 7: ADR 0038 + docs/README.md row

**Files:**

- Create: `docs/adr/0038-e2e-parallelism-via-per-test-identity-fixtures.md`
- Modify: `docs/README.md` (append a row to the ADR table)

**Interfaces:** none (documentation).

- [x] **Step 1: Write the ADR**

Create `docs/adr/0038-e2e-parallelism-via-per-test-identity-fixtures.md` with:

```markdown
# 0038. E2E parallelism via per-test identity fixtures + a serial project for global-singleton specs

- Status: accepted
- Date: 2026-06-29
- Issue: #61

## Context

The Nix-VM Playwright suite pinned `workers: 1`. The original justification was
SQLite read-then-write `SQLITE_BUSY` contention, resolved by `BEGIN IMMEDIATE`
(#18, #51/#52/#53). What remained were **logical races on shared global state**:
shared seeded accounts (`testlogin`/`testoperator`/`testnoemail`), the singleton
`site.title`/`base_url` that `admin-site` mutates and other specs assert, an
exact global-feed count in `posts`, and a global-newest (unfiltered) mail waiter.

## Decision

Parallel-safety is achieved per test, not per database:

1. **Per-test identity fixtures** (`end2end/tests/fixtures.ts`): `user`
   provisions a uniquely-named account out-of-band (so the test page stays
   logged out); `mailbox` is a recipient-scoped, cursor-tracked mail waiter
   bound to that account's unique address; `verifiedUser` adds the
   email-verification flow. Lazy and test-scoped, so each test that destructures
   them gets isolated state and the boilerplate lives once.

2. **One serial exception by config, not code.** `admin-site` mutates the
   singleton site identity, so each browser has a paired **serial** Playwright
   project (`${browser}-serial`, `testMatch` admin-site, `fullyParallel: false`)
   that `dependencies` on the parallel project and therefore runs in a
   non-overlapping phase. The title-asserting specs (`example`, `posts`) keep
   their `"jaunder.local"` assertions because the mutator can never overlap them.

3. **Own-scoped feed assertions.** `posts`' anonymous local-timeline test asserts
   a full first page (`toBeGreaterThanOrEqual`) and that pagination grows the
   set, rather than an exact global count.

VM capacity is raised to 4 cores / 4 GB so the 4 workers each get a vCPU.

## Consequences

- New specs follow the fixture-first pattern; a spec is added to the parallel
  project by default. Quarantine in a `-serial` project is reserved for genuine
  global-singleton mutation.
- Reaching _full_ isolation later means only de-globalizing `admin-site` (moving
  the title/base_url assertions out of `example`/`posts`, or restore-after) —
  every other spec is already isolated.
- The local `end2end/playwright.config.ts` is intentionally left diverging;
  deduping the two configs is #153.
```

- [x] **Step 2: Add the ADR table row in `docs/README.md`**

Read the ADR table (rows for 0000–0037) and append, immediately after the `0037` row, following the exact column format of the existing rows:

```markdown
| [0038](adr/0038-e2e-parallelism-via-per-test-identity-fixtures.md) | E2E parallelism via per-test identity fixtures + a serial project for global-singleton specs | accepted |
```

(Match the surrounding rows' column count and link style exactly — read an adjacent row first and mirror it.)

- [x] **Step 3: Format and commit**

```bash
prettier -w docs/README.md docs/adr/0038-e2e-parallelism-via-per-test-identity-fixtures.md
git add docs/adr/0038-e2e-parallelism-via-per-test-identity-fixtures.md docs/README.md
git commit -m "docs(adr): record e2e parallelism via per-test identity fixtures (0038, #61)"
```

---

### Task 8: Integration validation — Postgres-first, then SQLite, then repeats

**Files:** none (verification task). Deliverable: a green `cargo xtask validate` plus recorded evidence of repeated parallel stability.

**Why a dedicated task:** parallel-safety is flaky-shaped — a single green run is not proof. This task follows the spec's validation ladder and is the **behavioral gate** for Tasks 1-6.

- [ ] **Step 1: Postgres-first single-combo run (logical isolation, no lock concerns)**

Postgres is immune to SQLite write contention, so it isolates _logical_ races. Run the chromium Postgres combo first:

```bash
cargo xtask validate 2>&1 | tail -n 40
```

If a faster targeted path is preferred, build just the one combo:

```bash
nix build -L .#checks.x86_64-linux.e2e-postgres-chromium 2>&1 | tail -n 40
```

Expected: PASS. A failure here is a logical-isolation bug (a fixture or own-scoping miss), not a lock issue — debug with `superpowers:systematic-debugging` using the captured `build.log` + `playwright-report-postgres.json` (artifacts are captured pre-failure per #123/#49).

- [ ] **Step 2: SQLite combo run (confirms BEGIN IMMEDIATE holds under 4 workers)**

```bash
nix build -L .#checks.x86_64-linux.e2e-sqlite-chromium 2>&1 | tail -n 40
nix build -L .#checks.x86_64-linux.e2e-sqlite-firefox 2>&1 | tail -n 40
```

Expected: PASS. A `SQLITE_BUSY`/locking failure here means real multi-worker writes still contend — capture and investigate before proceeding.

- [ ] **Step 3: Repeat the fastest combo to shake out residual races**

A one-shot pass is insufficient evidence. Loop the fastest combo (sqlite-chromium) several times. Each `nix build` of an unchanged derivation is cached, so force a rerun:

```bash
for i in 1 2 3; do
  echo "=== parallel-stability run $i ==="
  nix build -L --rebuild .#checks.x86_64-linux.e2e-sqlite-chromium 2>&1 | tail -n 8
done
```

Expected: 3/3 PASS. Any intermittent failure is a residual race — do **not** proceed to the merge gate until it is root-caused and fixed (record which test, then debug).

- [ ] **Step 4: Full gate — all four `{sqlite,postgres}×{chromium,firefox}` combos**

```bash
cargo xtask validate 2>&1 | tail -n 40
```

Expected: full `validate` green (all combos + the non-e2e suite). This is the merge-readiness gate. Note any boot/infra flake (e.g. `/dev/net/tun` missing) is infra, not a logic failure — retry once per the project's known-flake guidance.

- [ ] **Step 5: Record the evidence (no commit — this is a verification gate)**

Summarize for the ship hand-off: Postgres-first result, SQLite result, the 3× repeat outcome, and the final `validate` status. This evidence is what the pre-merge halt point reviews.

---

## Self-Review

**Spec coverage:**

- Spec §1 (fixture foundation: `user`/`mailbox` recipient-scoped/`verifiedUser`) → **Task 1**. ✓ (mailbox is FIFO per-recipient via cursor; `user`/`verifiedUser` out-of-band to keep the test page logged out — a refinement the spec implies via "self-provision".)
- Spec §2 migrations: atompub/feeds/media/visibility/static-assets/unicode-slug/example → **no task needed** (already import from `./fixtures` and use `register()`; the spec's import-swap premise was stale). auth → **Task 4**; email → **Task 2**; password_reset → **Task 3**; posts own-scoping → **Task 5**. ✓
- Spec §3 (serial admin-site project, per-browser, `dependencies`, VM passes both projects) → **Task 6** steps 1-2. ✓ (admin-site keeps seeded `testoperator` — operator privilege can't be self-served; quarantine is by config.)
- Spec §4 (VM 4 cores / 4 GB; `fullyParallel`; `workers: 4`) → **Task 6** steps 1, 3, 4. ✓
- Spec §5 (validation: Postgres-first, SQLite, repeats, full validate) → **Task 8**. ✓
- Spec §Scope (don't dedupe configs = #153; update the `workers: 1` comment) → comment updated in **Task 6** step 1; local config untouched (Global Constraints). ✓
- Spec §ADR (0038 + docs/README.md row) → **Task 7**. ✓
- Spec §Acceptance (all six boxes) → covered by Tasks 1-8. ✓

**Placeholder scan:** No TBDs; every code step shows complete code. The `docs/README.md` row is exact; ADR body is complete.

**Type consistency:** `TestUser`/`Mailbox` defined in Task 1 are the exact types destructured in Tasks 2-5 (`user`, `mailbox`, `verifiedUser`). `user.password === "testpassword123"` matches `register()`'s hardcoded password. `mailbox.waitForNewEmail()` signature matches every call site. Project names `${browser}` / `${browser}-serial` in Task 6 step 2 match the `name:` fields in step 1.

**Deviations from spec (intentional, grounded in the actual code):**

1. The 6 "already-isolated" specs + `example` need **no edits** (they already use `./fixtures` + `register()`), so the spec's import-swap tasks collapse to zero work.
2. `admin-site` keeps its seeded `testoperator` and gets **no code change** — only config routing.
3. `user`/`verifiedUser` provision **out-of-band** (throwaway context) so login-form and password-reset tests start logged out — necessary because `register()` auto-logs-in.
4. `posts` change is **one test** (the guest local-timeline), not a broad migration — every other posts test is already own-scoped.

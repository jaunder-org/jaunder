/**
 * Shared helpers for Jaunder e2e tests.
 *
 * ## Usage rules
 *
 * - **A page boots once, at the URL under test** (#867).  That single document
 *   load is the page's entry; fixtures take the entry path rather than guessing
 *   it.  Everything after it moves within the app.
 *
 * - Always use `goto`, `click`, and `waitForSelector` from this module instead
 *   of `page.goto` / `page.click` / `page.waitForSelector` directly.  The
 *   wrappers record timing via `withTimedAction` so every navigation and
 *   interaction appears in the OTEL trace.  A `page.goto` anywhere under
 *   `end2end/tests` other than this module fails the `e2e-goto-wrapper` xtask
 *   check, unless the site carries a `// e2e-goto-wrapper:allow <reason>`
 *   marker on the line directly above it (ADR-0094).
 *
 * - **Move within the app with `navigateInApp`** (`./navigate`), not with a
 *   second `goto`: in a CSR SPA the router serves the move, and a document load
 *   exercises a path no user takes.  Its `ready` selector must not already
 *   match before the move — a barrier that waits for nothing is rejected, not
 *   silently accepted.
 *
 * - **A second document load on an already-booted page must be declared** with
 *   `allowSecondBoot(page, reason)` (`./bootBudget`), and the reason must be
 *   non-empty: it is the record of what was deliberately left alone (the
 *   destination's cold render being the subject, or a re-load proving
 *   persistence).  One allowance covers one load.  An allowance nothing
 *   consumes fails the test as an orphan — it does not expire, so leaving one
 *   behind would silently absorb the next undeclared load.  For the rare load
 *   whose very occurrence depends on the browser engine, and only for that,
 *   declare it with `allowEngineDependentBoot(page, path, reason)`: it covers at
 *   most one load **of that path** and is exempt from the orphan rule.  The path
 *   is what keeps an unconsumed one from absorbing some other load.
 *
 * - Pass paths (e.g. `"/login"`, `"/posts/new"`) to `goto` — it always
 *   prepends `BASE_URL` automatically.  Use `BASE_URL` directly only for
 *   non-`goto` calls such as `page.request.post`, `page.request.get`, and
 *   `page.waitForURL`.
 *
 * - `goto` waits for the CSR mount automatically.  Call
 *   `waitForMount(page)` only after action-triggered navigations (e.g.
 *   redirects from form submits, server-side 302s) where `goto` was not used.
 *   Do not call it after `navigateInApp` — the app is already mounted, so
 *   `body[data-mounted]` would pass vacuously.
 *
 * - Never use `page.waitForLoadState("networkidle")` — it fires before ActionForm
 *   AJAX responses arrive under load.  Wait for a specific element instead.
 *
 * - Whole-test timeout scaling is ambient (see `fixtures.ts`): every test gets a
 *   scaled `DEFAULT_TEST_BUDGET_MS` automatically, so tests do not hand-roll
 *   `test.setTimeout(slowBrowserTimeoutMs(...))`.  That budget covers every test
 *   in the suite (#270), so needing more is a signal, not a routine: measure the
 *   test first, and only then add a `setTestBudget(ms)` derived from whatever
 *   deadline actually exceeds the ambient budget.  Do not combine with
 *   `test.slow()` — the scaled budget already covers Firefox.
 *
 * - Use `signInAsNewUser(page)` whenever a test needs a fresh authenticated
 *   account, and `signInAs(page, username)` for an existing one (e.g. the
 *   harness-seeded `testoperator`). Both seed the account/session out-of-band
 *   and inject it into the context — no UI flow, no navigation (the test's own
 *   first `goto` is the cold navigation). `registerViaUi` / `login` /
 *   `fillLoginForm` are reserved for the holdouts whose subject IS the real
 *   flow (spec D6).
 */

import { expect, type Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { allowSecondBoot, throwIfViolated } from "./bootBudget";
import { extractLink, extractToken, type CapturedEmail } from "./mail";
import { waitForMount } from "./mount";
import {
  applySeededSession,
  createSessionViaTool,
  seedUserViaTool,
} from "./seed";
import { SEL } from "./selectors";

export { waitForMount } from "./mount";

// The server's base URL. `JAUNDER_E2E_BASE_URL` lets the harness point the suite
// at an ephemeral-port server (the host e2e loop feeds its discovered
// `http://ip:port`, #249); the Nix VM feeds nothing and keeps the fixed :3000.
export const BASE_URL =
  process.env.JAUNDER_E2E_BASE_URL ?? "http://localhost:3000";

// ---------------------------------------------------------------------------
// Low-level action wrappers
// ---------------------------------------------------------------------------

/**
 * Navigate to `path` (e.g. `"/login"`), prepending `BASE_URL` automatically,
 * and recording timing in the OTEL trace.
 */
export async function goto(
  page: Page,
  path: string,
  options?: Parameters<Page["goto"]>[1],
): Promise<void> {
  await withTimedAction(page, "page.goto", () =>
    // e2e-goto-wrapper:allow this call is the wrapper — the one document load that supplies the mount barrier every other site is required to go through
    page.goto(`${BASE_URL}${path}`, {
      waitUntil: "domcontentloaded",
      ...options,
    }),
  );
  await waitForMount(page, options?.timeout);
  // The budget's event handler cannot reject this promise, so it records the
  // breach and we raise it here (#867). Last, so a genuine mount failure — the
  // more informative error — wins.
  throwIfViolated(page);
}

/** Click `selector`, recording timing in the OTEL trace. */
export async function click(page: Page, selector: string): Promise<void> {
  await withTimedAction(page, "ui.click", () => page.click(selector));
}

/** Wait for `selector`, recording timing in the OTEL trace. */
export async function waitForSelector(
  page: Page,
  selector: string,
  options?: Parameters<Page["waitForSelector"]>[1],
): Promise<void> {
  await withTimedAction(page, "wait.selector", () =>
    options === undefined
      ? page.waitForSelector(selector)
      : page.waitForSelector(selector, options),
  );
}

// ---------------------------------------------------------------------------
// Fault injection
// ---------------------------------------------------------------------------

/**
 * Force a server-fn (`#[server(endpoint = "/vertical/op")]`, POSTed to
 * `/api/vertical/op`) to fail,
 * without touching the backend: Playwright fulfils the request in the browser with a 500,
 * so the client `Resource` resolves `Err` and the component's error branch renders.
 *
 * The server fn never executes — this exercises the *client* error UI only. Register the
 * route **before** the intercepted fetch fires (e.g. before `goto` for a page-load resource,
 * before creating the row whose child fetches for a nested one).
 */
export async function failServerFn(
  page: Page,
  endpoint: string,
): Promise<void> {
  await page.route(`**/api/${endpoint}`, (route) =>
    route.fulfill({ status: 500, body: "boom" }),
  );
}

/**
 * Hold every call to `/api/${endpoint}` open until the returned release fn runs, then
 * let it continue to the real backend.
 *
 * The releasable sibling of {@link failServerFn}: instead of fulfilling an immediate
 * 500, this suspends the request so a *loading* state can be observed
 * deterministically rather than raced. Register it **before** the action that triggers
 * the fetch, or the request escapes the route.
 *
 * Deliberately no `page.unroute` on release — unrouting immediately would race the
 * still-suspended handler's `continue()`, and the route is harmless once released.
 */
export async function stallServerFn(
  page: Page,
  endpoint: string,
): Promise<() => void> {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(`**/api/${endpoint}`, async (route) => {
    await gate;
    await route.continue();
  });
  return release;
}

// ---------------------------------------------------------------------------
// High-level flows
// ---------------------------------------------------------------------------

/**
 * Fill the login form (`username` / `password`) and submit — no navigation and
 * no success wait.  `login` builds on this after its `goto("/login")`; error-path
 * tests call it directly (after their own `goto`) and then assert on `SEL.error`.
 */
export async function fillLoginForm(
  page: Page,
  username: string,
  password: string,
): Promise<void> {
  await withTimedAction(page, "flow.fill_login_form", async () => {
    await page.fill(SEL.username, username);
    await page.fill(SEL.password, password);
    await click(page, SEL.submit);
  });
}

/**
 * Log in as `username` / `password` and wait until the sidebar logout link is
 * visible (confirming the shared session context has flipped to authenticated).
 *
 * Login redirects via client-side pushState now (#591), so `waitForURL` would be
 * reliable — but `SEL.logoutLink` is the better signal because it confirms auth
 * state (content readiness), not merely the URL.
 *
 * **Boots the page** (#867). An ADR-0098 holdout: the subject is the real login
 * flow, so the document load of `/login` stays. Callers whose page has already
 * booted must declare it with `allowSecondBoot` before calling — the declaration
 * belongs to the caller, since only the caller knows whether this is its entry.
 */
export async function login(
  page: Page,
  username: string,
  password: string,
  firstNavigationTimeoutMs?: number,
): Promise<void> {
  await withTimedAction(page, "flow.login", async () => {
    await goto(page, "/login", { timeout: firstNavigationTimeoutMs });
    await fillLoginForm(page, username, password);
    await waitForSelector(page, SEL.logoutLink);
  });
}

/** The fixed password every seeded account gets (spec D4). Kept as one
 *  constant so the seeded helpers and the fixtures agree. */
export const TEST_PASSWORD = "testpassword123";

/** `user1754…`-style unique usernames; `prefix` distinguishes invitees etc.
 *  Stays in TypeScript (spec D4): the caller needs the name before the call
 *  returns, and the per-user-unique scheme is what `seedPostsViaTool`'s
 *  per-user slug uniqueness relies on. */
export function generateUsername(prefix = "user"): string {
  return `${prefix}${Date.now()}${Math.random().toString(36).slice(2, 8)}`;
}

/**
 * Seed a fresh account + session out-of-band and inject it into `page`'s
 * context, returning the generated username. No UI flow and NO navigation
 * (spec D5): the test's own first `goto` becomes the cold navigation, saving
 * a whole page load on top of the form and submit.
 */
export async function signInAsNewUser(page: Page): Promise<string> {
  const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
  await applySeededSession(page.context(), record);
  return record.username;
}

/**
 * Same as {@link signInAsNewUser}, returning the fixed password too — for
 * tests that re-drive the account across contexts (`signInAs`) or through the
 * login form.
 */
export async function signInAsNewUserKnown(
  page: Page,
): Promise<{ username: string; password: string }> {
  const username = await signInAsNewUser(page);
  return { username, password: TEST_PASSWORD };
}

/**
 * Seed a session for an EXISTING account (e.g. the harness-seeded
 * `testoperator` / `testlogin`) and inject it into `page`'s context. No UI
 * flow and NO navigation (spec D5).
 */
export async function signInAs(
  page: Page,
  username: string,
  label?: string,
): Promise<void> {
  const record = await createSessionViaTool(username, label);
  await applySeededSession(page.context(), record);
}

/**
 * The real UI registration flow — reserved for the holdouts whose subject is
 * that flow (spec D6). Emits the timed action under the unchanged name
 * `flow.register` so trace counts stay comparable.
 *
 * After submission the helper races between `a[href='/logout']` (success) and
 * `.error` (failure) for fast failure detection.
 *
 * **Boots the page** (#867), on the same terms as {@link login}: an ADR-0098
 * holdout whose document load of `/register` is the subject. A caller whose page
 * has already booted declares it with `allowSecondBoot`.
 */
export async function registerViaUi(
  page: Page,
  firstNavigationTimeoutMs: number,
): Promise<string> {
  const username = generateUsername();

  await withTimedAction(page, "flow.register", async () => {
    await goto(page, "/register", { timeout: firstNavigationTimeoutMs });
    await withTimedAction(page, "ui.fill.username", () =>
      page.fill(SEL.username, username),
    );
    await withTimedAction(page, "ui.fill.password", () =>
      page.fill(SEL.password, TEST_PASSWORD),
    );
    await click(page, SEL.submit);

    // Race success marker vs explicit server error so we fail fast on
    // misconfiguration rather than burning the full test timeout.
    const outcome = await Promise.race([
      page
        .waitForSelector(SEL.logoutLink, { timeout: 10_000 })
        .then(() => "ok"),
      page.waitForSelector(SEL.error, { timeout: 10_000 }).then(() => "error"),
    ]);
    if (outcome === "error") {
      const errorText = (
        await page.locator(SEL.error).first().textContent()
      )?.trim();
      throw new Error(`registration failed: ${errorText ?? "unknown error"}`);
    }
  });

  return username;
}

/**
 * The minimal recipient-scoped mail waiter this module needs.
 *
 * Structural rather than an import of `fixtures.ts`'s `Mailbox`: `fixtures.ts`
 * imports this module, so naming its type here would close a cycle.
 */
export type EmailWaiter = {
  waitForNewEmail(timeoutMs?: number): Promise<CapturedEmail>;
};

/**
 * Set the current (authenticated) page's email address and complete the
 * verification round trip: submit the address, read the token out of the
 * captured mail, and follow the verify link.
 *
 * Was written inline in the `verifiedUser` fixture and again in the email spec.
 * Factored so the phases are delimited in the trace and the sequence has one
 * home (#794).
 *
 * **Two document loads** (#867): `/profile/email` is the page's entry for both
 * current callers, and following the emailed link is a second load this helper
 * declares itself — it is an arrival from outside the app either way, so the
 * declaration does not depend on the caller.
 */
export async function setAndVerifyEmail(
  page: Page,
  email: string,
  mailbox: EmailWaiter,
): Promise<void> {
  await withTimedAction(page, "flow.verify_email", async () => {
    await goto(page, "/profile/email");
    await page.fill('input[name="email"]', email);
    await click(page, SEL.submit);
    await expectFlash(page, "Check your email");
    const mail = await mailbox.waitForNewEmail();
    const token = extractToken(mail);
    allowSecondBoot(
      page,
      "following the emailed verification link is an arrival from outside the app, exactly as a real recipient does",
    );
    await goto(page, `/verify-email?token=${token}`);
    await expectFlash(page, "verified");
  });
}

/**
 * Submit `/forgot-password` for `username`.
 *
 * Deliberately makes no assertion about the response: callers assert different
 * things about it — the happy path expects a neutral confirmation that does not
 * reveal whether the user exists, while the no-verified-email path expects an
 * error — so the shared part stops at the submit.
 *
 * **Boots the page** (#867): `/forgot-password` is reached from outside a
 * session, and for both current callers it is the page's entry.
 */
export async function requestPasswordReset(
  page: Page,
  username: string,
): Promise<void> {
  await withTimedAction(page, "flow.request_password_reset", async () => {
    await goto(page, "/forgot-password");
    await page.fill(SEL.username, username);
    await click(page, SEL.submit);
  });
}

/**
 * Click one side of the subscribe toggle and settle on an outcome.
 *
 * Waits for *either* the flipped button or an error, then decides — rather than
 * waiting only for the flipped button. #861 is why: a failed subscription check
 * used to repaint the opposite button, so "the button flipped" was satisfied by
 * both success and failure, and the caller carried on believing the write had
 * committed. The subsequent assertion then failed somewhere else entirely,
 * looking indistinguishable from a privacy regression.
 *
 * The component now paints `.error` on either a failed check or a failed
 * mutation, so a failure that once masqueraded as success surfaces here, named,
 * at the step that actually broke.
 */
async function toggleSubscription(
  page: Page,
  authorUsername: string,
  clickLabel: string,
  settledLabel: string,
): Promise<void> {
  await goto(page, `/~${authorUsername}`);
  await click(page, `button:has-text("${clickLabel}")`);

  const settled = page.locator(`button:has-text("${settledLabel}")`);
  const failed = page.locator(SEL.error);
  await expect(settled.or(failed).first()).toBeVisible();

  if ((await failed.count()) > 0) {
    const detail = (await failed.first().innerText()).trim();
    throw new Error(
      `${clickLabel} for ~${authorUsername} failed: ${detail}. ` +
        `The write did not commit, so any later visibility assertion would be ` +
        `testing the wrong thing (#861).`,
    );
  }
  await expect(settled).toBeVisible();
}

/**
 * Subscribe the current (authenticated) page's user to `authorUsername` via the
 * author's profile page, settling once the button flips to "Unsubscribe".
 *
 * **Boots the page** (#867): nothing in the app links to an arbitrary author's
 * profile from wherever the caller happens to be, so this stays a document load.
 * For most callers it is a freshly created page's entry; a caller whose page has
 * already booted declares it with `allowSecondBoot` first.
 */
export async function subscribeTo(
  page: Page,
  authorUsername: string,
): Promise<void> {
  await withTimedAction(page, "flow.subscribe", () =>
    toggleSubscription(page, authorUsername, "Subscribe", "Unsubscribe"),
  );
}

/**
 * Unsubscribe the current page's user from `authorUsername` via the profile
 * page, settling once the button flips back to "Subscribe".
 *
 * **Boots the page** (#867), on the same terms as {@link subscribeTo}.
 */
export async function unsubscribeFrom(
  page: Page,
  authorUsername: string,
): Promise<void> {
  await withTimedAction(page, "flow.unsubscribe", () =>
    toggleSubscription(page, authorUsername, "Unsubscribe", "Subscribe"),
  );
}

/**
 * Assert that a confirmation flash `<p>` containing `text` becomes visible,
 * standardising the `expect(locator('p:has-text(...)')).toBeVisible()` idiom and
 * its ad-hoc timeout.
 */
export async function expectFlash(
  page: Page,
  text: string,
  timeout?: number,
): Promise<void> {
  const options = timeout === undefined ? {} : { timeout };
  await expect(page.locator(`p:has-text("${text}")`)).toBeVisible(options);
}

/**
 * Follow a token-bearing link from a captured email on the live test server.
 *
 * Asserts the emitted link is **absolute** (composed from the seeded
 * `site.base_url` `https://example.com`) — a relative `/…?token=` is unusable in
 * a real mail client, so this catches a relative-link regression — then re-bases
 * the link's own path onto the running server (the seeded base URL is
 * deliberately not the test server's address) and navigates via `goto`.
 * `pathPrefix` is the expected URL path, e.g. `"/reset-password"`.
 */
export async function followEmailLink(
  page: Page,
  email: CapturedEmail,
  pathPrefix: string,
): Promise<void> {
  await withTimedAction(page, "flow.follow_email_link", async () => {
    const link = extractLink(email);
    expect(link).toMatch(
      new RegExp(`^https://example\\.com${pathPrefix}\\?token=`),
    );
    const { pathname, search } = new URL(link);
    allowSecondBoot(
      page,
      "following the emailed reset link is an arrival from outside the app, exactly as a real recipient does",
    );
    await goto(page, `${pathname}${search}`);
  });
}

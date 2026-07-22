/**
 * Shared helpers for Jaunder e2e tests.
 *
 * ## Usage rules
 *
 * - Always use `goto`, `click`, and `waitForSelector` from this module instead
 *   of `page.goto` / `page.click` / `page.waitForSelector` directly.  The
 *   wrappers record timing via `withTimedAction` so every navigation and
 *   interaction appears in the OTEL trace.
 *
 * - Pass paths (e.g. `"/login"`, `"/posts/new"`) to `goto` — it always
 *   prepends `BASE_URL` automatically.  Use `BASE_URL` directly only for
 *   non-`goto` calls such as `page.request.post`, `page.request.get`, and
 *   `page.waitForURL`.
 *
 * - `goto` waits for Leptos WASM hydration automatically.  Call
 *   `waitForHydration(page)` only after action-triggered navigations (e.g.
 *   redirects from form submits, server-side 302s) where `goto` was not used.
 *
 * - Never use `page.waitForLoadState("networkidle")` — it fires before ActionForm
 *   AJAX responses arrive under load.  Wait for a specific element instead.
 *
 * - Whole-test timeout scaling is ambient (see `fixtures.ts`): every test gets a
 *   scaled `DEFAULT_TEST_BUDGET_MS` automatically, so tests no longer hand-roll
 *   `test.setTimeout(slowBrowserTimeoutMs(...))`.  A test needing a larger budget
 *   calls `setTestBudget(ms)` as its first line.  Do not combine with
 *   `test.slow()` — the scaled budget already covers Firefox.
 *
 * - Use `login(page, username, password)` for any test that needs an
 *   authenticated session; use `fillLoginForm(...)` (fill + submit, no wait)
 *   directly when exercising the login/error path.
 *
 * - Use `register(page, firstNavigationTimeoutMs)` whenever a test needs a
 *   fresh user account.  Pass `slowBrowserFirstNavigationTimeoutMs(...)` as
 *   the timeout so the cold WASM load gets enough budget across all browsers.
 */

import { expect, type Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { waitForHydration } from "./hydration";
import { SEL } from "./selectors";

export { waitForHydration } from "./hydration";

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
    page.goto(`${BASE_URL}${path}`, {
      waitUntil: "domcontentloaded",
      ...options,
    }),
  );
  await waitForHydration(page, options?.timeout);
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
 * Force a server-fn (`#[server(endpoint = "/name")]`, POSTed to `/api/name`) to fail,
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
  await page.fill(SEL.username, username);
  await page.fill(SEL.password, password);
  await click(page, SEL.submit);
}

/**
 * Log in as `username` / `password` and wait until the sidebar logout link is
 * visible (confirming the shared session context has flipped to authenticated).
 *
 * Login redirects via client-side pushState now (#591), so `waitForURL` would be
 * reliable — but `SEL.logoutLink` is the better signal because it confirms auth
 * state (content readiness), not merely the URL.
 */
export async function login(
  page: Page,
  username: string,
  password: string,
  firstNavigationTimeoutMs?: number,
): Promise<void> {
  await goto(page, "/login", { timeout: firstNavigationTimeoutMs });
  await fillLoginForm(page, username, password);
  await waitForSelector(page, SEL.logoutLink);
}

/**
 * Register a new user with a unique generated username, wait for the login
 * redirect to settle, and return the username.
 *
 * Pass `slowBrowserFirstNavigationTimeoutMs(testInfo, ms)` as
 * `firstNavigationTimeoutMs` to give the cold WASM load enough budget on all
 * browsers.
 *
 * After submission the helper races between `a[href='/logout']` (success) and
 * `.error` (failure) for fast failure detection.
 */
export async function register(
  page: Page,
  firstNavigationTimeoutMs: number,
): Promise<string> {
  const username = `user${Date.now()}${Math.random().toString(36).slice(2, 8)}`;

  await withTimedAction(page, "flow.register", async () => {
    await goto(page, "/register", { timeout: firstNavigationTimeoutMs });
    await withTimedAction(page, "ui.fill.username", () =>
      page.fill(SEL.username, username),
    );
    await withTimedAction(page, "ui.fill.password", () =>
      page.fill(SEL.password, "testpassword123"),
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
 * Register a fresh user and return both the generated username and the fixed
 * password `register` sets, so the account can be re-driven across browser
 * contexts via `login`.
 */
export async function registerKnown(
  page: Page,
  firstNavigationTimeoutMs: number,
): Promise<{ username: string; password: string }> {
  const username = await register(page, firstNavigationTimeoutMs);
  return { username, password: "testpassword123" };
}

/**
 * Subscribe the current (authenticated) page's user to `authorUsername` via the
 * author's profile page, waiting for the button to flip to "Unsubscribe".
 */
export async function subscribeTo(
  page: Page,
  authorUsername: string,
): Promise<void> {
  await goto(page, `/~${authorUsername}`);
  await click(page, 'button:has-text("Subscribe")');
  await waitForSelector(page, 'button:has-text("Unsubscribe")');
}

/**
 * Unsubscribe the current page's user from `authorUsername` via the profile
 * page, waiting for the button to flip back to "Subscribe".
 */
export async function unsubscribeFrom(
  page: Page,
  authorUsername: string,
): Promise<void> {
  await goto(page, `/~${authorUsername}`);
  await click(page, 'button:has-text("Unsubscribe")');
  await waitForSelector(page, 'button:has-text("Subscribe")');
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

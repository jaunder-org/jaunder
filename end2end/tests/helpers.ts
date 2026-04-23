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
 * - Call `waitForHydration(page)` after every navigation before filling any
 *   form fields.  Leptos `prop:value` bindings reset input values during WASM
 *   hydration; filling before hydration completes sends empty fields to the
 *   server.
 *
 * - Never use `page.waitForLoadState("networkidle")` — it fires before
 *   Firefox's `location.replace()` navigation and before ActionForm AJAX
 *   responses arrive under load.  Wait for a specific element instead.
 *
 * - Use `hydrationHeavyTimeoutMs(testInfo, ms)` (from fixtures) for all test
 *   timeouts so Firefox gets a scaled budget.  Do not combine it with
 *   `test.slow()` — the explicit timeout wins and `test.slow()` is redundant.
 *
 * - Use `login(page, username, password)` for any test that needs an
 *   authenticated session.
 *
 * - Use `register(page, firstNavigationTimeoutMs)` whenever a test needs a
 *   fresh user account.  Pass `hydrationHeavyFirstNavigationTimeoutMs(...)` as
 *   the timeout so the cold WASM load gets enough budget across all browsers.
 */

import type { Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { waitForHydration } from "./hydration";

export { waitForHydration } from "./hydration";

export const BASE_URL = "http://localhost:3000";

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
    page.goto(`${BASE_URL}${path}`, options),
  );
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
// High-level flows
// ---------------------------------------------------------------------------

/**
 * Log in as `username` / `password` and wait until the sidebar logout link is
 * visible (confirming the authenticated Suspense has resolved).
 *
 * Do NOT use `waitForURL` here — it is unreliable in Firefox for
 * `location.replace()` navigations.  Waiting for `a[href='/logout']` is the
 * correct signal because it is only rendered once auth state is confirmed.
 */
export async function login(
  page: Page,
  username: string,
  password: string,
  firstNavigationTimeoutMs?: number,
): Promise<void> {
  await goto(page, "/login", {
    waitUntil: "domcontentloaded",
    timeout: firstNavigationTimeoutMs,
  });
  await waitForHydration(page, firstNavigationTimeoutMs);
  await page.fill('input[name="username"]', username);
  await page.fill('input[name="password"]', password);
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, "a[href='/logout']");
}

/**
 * Register a new user with a unique generated username, wait for the login
 * redirect to settle, and return the username.
 *
 * Pass `hydrationHeavyFirstNavigationTimeoutMs(testInfo, ms)` as
 * `firstNavigationTimeoutMs` to give the cold WASM load enough budget on all
 * browsers.
 *
 * After submission the helper races between `a[href='/logout']` (success) and
 * `.error` (failure) rather than using `waitForLoadState("networkidle")`,
 * which fires prematurely in Firefox.
 */
export async function register(
  page: Page,
  firstNavigationTimeoutMs: number,
): Promise<string> {
  const username = `user${Date.now()}${Math.random().toString(36).slice(2, 8)}`;

  await goto(page, "/register", {
    timeout: firstNavigationTimeoutMs,
  });
  await waitForHydration(page, firstNavigationTimeoutMs);
  await withTimedAction(page, "ui.fill.username", () =>
    page.fill('input[name="username"]', username),
  );
  await withTimedAction(page, "ui.fill.password", () =>
    page.fill('input[name="password"]', "testpassword123"),
  );
  await click(page, 'button[type="submit"]');

  // Race success marker vs explicit server error so we fail fast on
  // misconfiguration rather than burning the full test timeout.
  const outcome = await Promise.race([
    page
      .waitForSelector("a[href='/logout']", { timeout: 10_000 })
      .then(() => "ok"),
    page.waitForSelector(".error", { timeout: 10_000 }).then(() => "error"),
  ]);
  if (outcome === "error") {
    const errorText = (
      await page.locator(".error").first().textContent()
    )?.trim();
    throw new Error(`registration failed: ${errorText ?? "unknown error"}`);
  }

  return username;
}

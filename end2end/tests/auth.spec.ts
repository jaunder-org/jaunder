import { test, expect, type Page } from "@playwright/test";

/** Wait for Leptos WASM hydration to complete before interacting with forms.
 *
 * Leptos sets `prop:value` bindings on controlled inputs during hydration,
 * which resets any values filled before hydration completes. Waiting for
 * `data-hydrated` (set by the WASM `hydrate()` function after
 * `hydrate_body()` returns) ensures all reactive effects have run before
 * Playwright fills form fields.
 */
async function waitForHydration(page: Page): Promise<void> {
  await page.waitForSelector("body[data-hydrated]");
}

test("register page shows form", async ({ page }) => {
  await page.goto("http://localhost:3000/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("register with open policy succeeds", async ({ page }) => {
  const username = `newuser${Date.now()}${Math.random().toString(36).slice(2, 8)}`;
  await page.goto("http://localhost:3000/register");
  await waitForHydration(page);

  await page.fill('input[name="username"]', username);
  await page.fill('input[name="password"]', "newpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".error")).not.toBeVisible();
});

test("login page shows form", async ({ page }) => {
  await page.goto("http://localhost:3000/login");

  await expect(page.locator("h1")).toHaveText("Login");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("login with valid credentials succeeds", async ({ page }) => {
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  // waitForURL is unreliable in Firefox for location.replace() navigations; wait
  // for the logout link that SSR renders on the home page after successful auth.
  await page.waitForSelector("a[href='/logout']");
  await waitForHydration(page);

  await expect(page.locator("header")).toContainText("Logged in as testlogin");
  await expect(page.locator("header a[href='/logout']")).toBeVisible();
});

test("login with wrong password shows error", async ({ page }) => {
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "wrongpassword!");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".error")).toBeVisible();
});

test("logout page logs out", async ({ page }) => {
  // Log in first to establish a session
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  // Now navigate to logout
  await page.goto("http://localhost:3000/logout");

  await expect(page.locator("h1")).toContainText("Logging out");
  await page.waitForLoadState("networkidle");

  await expect(page.locator("p")).toContainText("You have been logged out.");
});

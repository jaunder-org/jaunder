import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { createPerfProbe } from "./perf";
import { waitForHydration } from "./hydration";

test("register page shows form", async ({ page }, testInfo) => {
  test.slow();
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await page.goto("http://localhost:3000/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("register with open policy succeeds", async ({ page }, testInfo) => {
  test.slow();
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  const username = `newuser${Date.now()}${Math.random().toString(36).slice(2, 8)}`;
  await page.goto("http://localhost:3000/register");
  await waitForHydration(page);

  await page.fill('input[name="username"]', username);
  await page.fill('input[name="password"]', "newpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".error")).not.toBeVisible();
});

test("login page shows form", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await page.goto("http://localhost:3000/login");

  await expect(page.locator("h1")).toHaveText("Login");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("login with valid credentials succeeds", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  const perf = createPerfProbe(testInfo, "auth_login_success");

  perf.mark("goto_login_start");
  await page.goto("http://localhost:3000/login");
  perf.mark("goto_login_done");
  await waitForHydration(page);
  perf.mark("hydration_done");

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  perf.mark("credentials_filled");
  await page.click('button[type="submit"]');
  perf.mark("submit_clicked");
  // waitForURL is unreliable in Firefox for location.replace() navigations; wait
  // for the sidebar logout link, which only appears after the Suspense resolves
  // with the authenticated state — by that point the navigation is fully settled.
  await page.waitForSelector("a[href='/logout']");
  perf.mark("logout_link_visible");
  await waitForHydration(page);
  perf.mark("post_login_hydration_done");

  await expect(page.locator(".j-sb-foot")).toContainText("testlogin");
  await expect(page.locator(".j-sidebar")).toBeVisible();
  perf.mark("assertions_complete");
  await perf.log();
});

test("login with wrong password shows error", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "wrongpassword!");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".error")).toBeVisible();
});

test("logout page logs out", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  // Log in first to establish a session
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForSelector("a[href='/logout']");

  // Use the rendered logout link to avoid Firefox navigation abort races.
  await page.click("a[href='/logout']");

  // Logout clears the session and redirects to "/"; wait for the sidebar footer
  // to reflect the unauthenticated state on the home page.
  await page.waitForSelector(".j-sb-foot");
  await waitForHydration(page);
  await expect(page.locator(".j-sb-foot")).toContainText("Sign in");
});

test("sidebar reverts to sign-in after logout", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  // Log in and confirm the sidebar shows the username.
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForSelector("a[href='/logout']");
  // a[href='/logout'] only renders when auth Suspense resolves, confirming testlogin is shown.
  await expect(page.locator(".j-sb-foot")).toContainText("testlogin");

  // Click the sidebar "Sign out" link and confirm the sidebar switches back.
  await page.click("a[href='/logout']");
  // Logout is a server-side 302 redirect (not location.replace), so waitForURL is reliable.
  await page.waitForURL("http://localhost:3000/", { timeout: 10_000 });
  await waitForHydration(page);
  await expect(page.locator(".j-sb-foot")).not.toContainText("testlogin");
  await expect(page.locator(".j-sb-foot")).toContainText("Sign in");
});

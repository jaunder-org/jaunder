import { test, expect } from "./fixtures";
import { createPerfProbe } from "./perf";
import { waitForHydration } from "./hydration";

test("register page shows form", async ({ page }) => {
  test.slow();
  await page.goto("http://localhost:3000/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("register with open policy succeeds", async ({ page }) => {
  test.slow();
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

test("login with valid credentials succeeds", async ({ page }, testInfo) => {
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
  // for the logout link that SSR renders on the home page after successful auth.
  await page.waitForSelector("a[href='/logout']");
  perf.mark("logout_link_visible");
  await waitForHydration(page);
  perf.mark("post_login_hydration_done");

  await expect(page.locator("header")).toContainText("Logged in as testlogin");
  await expect(page.locator("header a[href='/logout']")).toBeVisible();
  perf.mark("assertions_complete");
  await perf.log();
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
  await page.waitForSelector("a[href='/logout']");

  // Use the rendered logout link to avoid Firefox navigation abort races.
  await page.click("a[href='/logout']");

  await expect(page.locator("h1")).toContainText("Logging out");
  await page.waitForLoadState("networkidle");

  await expect(page.locator("p")).toContainText("You have been logged out.");
});

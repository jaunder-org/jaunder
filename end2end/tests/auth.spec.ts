import {
  test,
  expect,
  hydrationHeavyTimeoutMs,
  hydrationHeavyFirstNavigationTimeoutMs,
} from "./fixtures";
import { createPerfProbe } from "./perf";
import {
  BASE_URL,
  goto,
  click,
  waitForSelector,
  waitForHydration,
  login,
  register,
} from "./helpers";

test("register page shows form", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await goto(page, "/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("register with open policy succeeds", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  const username = `newuser${Date.now()}${Math.random().toString(36).slice(2, 8)}`;
  await goto(page, "/register");

  await page.fill('input[name="username"]', username);
  await page.fill('input[name="password"]', "newpassword123");
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, "a[href='/logout']");

  await expect(page.locator(".error")).not.toBeVisible();
});

test("login page shows form", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await goto(page, "/login");

  await expect(page.locator("h1")).toHaveText("Login");
  await expect(page.locator('input[name="username"]')).toBeVisible();
  await expect(page.locator('input[name="password"]')).toBeVisible();
});

test("login with valid credentials succeeds", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  const perf = createPerfProbe(testInfo, "auth_login_success");

  await goto(page, "/login");

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  perf.mark("credentials_filled");
  await click(page, 'button[type="submit"]');
  perf.mark("submit_clicked");
  // waitForURL is unreliable in Firefox for location.replace() navigations; wait
  // for the sidebar logout link, which only appears after the Suspense resolves
  // with the authenticated state — by that point the navigation is fully settled.
  await waitForSelector(page, "a[href='/logout']");
  perf.mark("logout_link_visible");
  await waitForHydration(page);

  await expect(page.locator(".j-sb-foot")).toContainText("testlogin");
  await expect(page.locator(".j-sidebar")).toBeVisible();
  perf.mark("assertions_complete");
  await perf.log();
});

test("login with wrong password shows error", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await goto(page, "/login");

  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "wrongpassword!");
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, ".error");

  await expect(page.locator(".error")).toBeVisible();
});

test("logout page logs out", async ({ page }, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await login(page, "testlogin", "testpassword123");

  // Use the rendered logout link to avoid Firefox navigation abort races.
  await click(page, "a[href='/logout']");

  // Logout clears the session and redirects to "/"; waitForURL is reliable here
  // because logout is a server-side 302 redirect (not location.replace).
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await waitForHydration(page);
  // Footer shows neither username nor sign-in link after logout.
  await expect(page.locator(".j-sb-foot")).not.toContainText("testlogin");
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("sidebar reverts to signed-out state after logout", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));
  await login(page, "testlogin", "testpassword123");
  // a[href='/logout'] only renders when auth Suspense resolves, confirming testlogin is shown.
  await expect(page.locator(".j-sb-foot")).toContainText("testlogin");

  // Click the sidebar "Sign out" link and confirm the sidebar switches back.
  await click(page, "a[href='/logout']");
  // Logout is a server-side 302 redirect (not location.replace), so waitForURL is reliable.
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await waitForHydration(page);
  await expect(page.locator(".j-sb-foot")).not.toContainText("testlogin");
  // Footer no longer shows a Sign-in link — it renders nothing when unauthenticated.
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("sidebar shows only Home nav link when not logged in", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await goto(page, "/", {
    timeout: hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  });

  // Wait for the nav Suspense to resolve.
  await waitForSelector(page, ".j-nav");

  // Only one <a> inside .j-nav — the Home link.
  const navAnchors = page.locator(".j-nav a");
  await expect(navAnchors).toHaveCount(1);
  await expect(navAnchors.first()).toHaveAttribute("href", "/");

  // Sidebar footer must not contain a "Sign in" link.
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("sidebar footer shows Sign out link when logged in", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await waitForSelector(page, ".j-nav");
  // Still only Home in the nav (others have no hrefs yet).
  await expect(page.locator(".j-nav a")).toHaveCount(1);

  // Footer has Sign out.
  await expect(page.locator("a[href='/logout']")).toBeVisible();
  // Footer does NOT have Sign in.
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

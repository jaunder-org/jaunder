import { test, expect } from "./fixtures";
import { createPerfProbe } from "./perf";
import {
  BASE_URL,
  goto,
  click,
  waitForSelector,
  waitForHydration,
  login,
  fillLoginForm,
} from "./helpers";
import { SEL } from "./selectors";

test("register page shows form", async ({ page }) => {
  await goto(page, "/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator(SEL.username)).toBeVisible();
  await expect(page.locator(SEL.password)).toBeVisible();
});

test("register rejects a too-short password client-side", async ({ page }) => {
  await goto(page, "/register");

  await page.fill(SEL.username, "validusername");
  await page.fill(SEL.password, "short"); // < 8 chars
  await page.locator(SEL.password).blur(); // touched → message shows

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();

  // A valid password clears the error and enables submit.
  await page.fill(SEL.password, "longenough123");
  await expect(page.locator(SEL.submit)).toBeEnabled();
});

test("register with open policy succeeds", async ({ page }) => {
  const username = `newuser${Date.now()}${Math.random().toString(36).slice(2, 8)}`;
  await goto(page, "/register");

  await page.fill(SEL.username, username);
  await page.fill(SEL.password, "newpassword123");
  await click(page, SEL.submit);
  await waitForSelector(page, SEL.logoutLink);

  await expect(page.locator(SEL.error)).not.toBeVisible();
});

test("login page shows form", async ({ page }) => {
  await goto(page, "/login");

  await expect(page.locator("h1")).toHaveText("Login");
  await expect(page.locator(SEL.username)).toBeVisible();
  await expect(page.locator(SEL.password)).toBeVisible();
});

test("login with valid credentials succeeds", async ({
  page,
  user,
}, testInfo) => {
  const perf = createPerfProbe(testInfo, "auth_login_success");

  await goto(page, "/login");

  await page.fill(SEL.username, user.username);
  await page.fill(SEL.password, user.password);
  perf.mark("credentials_filled");
  await click(page, SEL.submit);
  perf.mark("submit_clicked");
  // waitForURL is unreliable in Firefox for location.replace() navigations; wait
  // for the sidebar logout link, which only appears after the Suspense resolves
  // with the authenticated state — by that point the navigation is fully settled.
  await waitForSelector(page, SEL.logoutLink);
  perf.mark("logout_link_visible");
  await waitForHydration(page);

  await expect(page.locator(".j-sb-foot")).toContainText(user.username);
  await expect(page.locator(".j-sidebar")).toBeVisible();
  perf.mark("assertions_complete");
  await perf.log();
});

test("login with wrong password shows error", async ({ page }) => {
  await goto(page, "/login");

  await fillLoginForm(page, "testlogin", "wrongpassword!");
  await waitForSelector(page, SEL.error);

  await expect(page.locator(SEL.error)).toBeVisible();
});

test("logout page logs out", async ({ page, user }) => {
  await login(page, user.username, user.password);

  // Use the rendered logout link to avoid Firefox navigation abort races.
  await click(page, SEL.logoutLink);

  // Logout clears the session and redirects to "/"; waitForURL is reliable here
  // because logout is a server-side 302 redirect (not location.replace).
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await waitForHydration(page);
  // Footer shows neither username nor sign-in link after logout.
  await expect(page.locator(".j-sb-foot")).not.toContainText(user.username);
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("sidebar reverts to signed-out state after logout", async ({
  page,
  user,
}) => {
  await login(page, user.username, user.password);
  // a[href='/logout'] only renders when auth Suspense resolves, confirming the
  // user is shown.
  await expect(page.locator(".j-sb-foot")).toContainText(user.username);

  // Click the sidebar "Sign out" link and confirm the sidebar switches back.
  await click(page, SEL.logoutLink);
  // Logout is a server-side 302 redirect (not location.replace), so waitForURL is reliable.
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await waitForHydration(page);
  await expect(page.locator(".j-sb-foot")).not.toContainText(user.username);
  // Footer no longer shows a Sign-in link — it renders nothing when unauthenticated.
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("sidebar shows only Home nav link when not logged in", async ({
  page,
  firstNav,
}) => {
  await goto(page, "/", {
    timeout: firstNav,
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
  registeredPage: page,
}) => {
  // Wait for the authenticated nav to render from the marker (#181 — synchronous,
  // no Suspense swap).
  await waitForSelector(page, ".j-nav a[href='/drafts']");
  // Home, Feed (/app cockpit, #181), Drafts, Media, and Audiences have hrefs.
  await waitForSelector(page, ".j-nav a[href='/audiences']");
  await expect(page.locator(".j-nav a")).toHaveCount(5);

  // Footer has Sign out.
  await expect(page.locator(SEL.logoutLink)).toBeVisible();
  // Footer does NOT have Sign in.
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

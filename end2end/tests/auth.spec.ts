import { test, expect } from "./fixtures";
import { createPerfProbe } from "./perf";
import {
  BASE_URL,
  goto,
  click,
  waitForSelector,
  signInAs,
  fillLoginForm,
  failServerFn,
  stallServerFn,
} from "./helpers";
import { SEL } from "./selectors";

test("register page shows form", async ({ page }) => {
  // Holdout (spec D6): proves /register renders.
  await goto(page, "/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator(SEL.username)).toBeVisible();
  await expect(page.locator(SEL.password)).toBeVisible();
});

test("register rejects a too-short password client-side", async ({ page }) => {
  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/registration/register")) requests += 1;
  });
  // Holdout (spec D6): proves client-side validation.
  await goto(page, "/register");

  await page.fill(SEL.username, "validusername");
  await page.fill(SEL.password, "short"); // < 8 chars
  await page.locator(SEL.password).blur(); // touched → message shows

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await page.locator(SEL.password).press("Enter");
  expect(requests).toBe(0);

  // A valid password clears the error and enables submit.
  await page.fill(SEL.password, "longenough123");
  await expect(page.locator(SEL.submit)).toBeEnabled();
});

test("register pending state prevents duplicate dispatch", async ({ page }) => {
  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/registration/register")) requests += 1;
  });
  await goto(page, "/register");
  const release = await stallServerFn(page, "registration/register");
  await page.fill(SEL.username, `pending${Date.now()}`);
  await page.fill(SEL.password, "newpassword123");
  await click(page, SEL.submit);
  await expect.poll(() => requests).toBe(1);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await page.locator(SEL.password).press("Enter");
  expect(requests).toBe(1);
  release();
  await waitForSelector(page, SEL.logoutLink);
});

test("register server failure renders error", async ({ page }) => {
  await failServerFn(page, "registration/register");
  await goto(page, "/register");
  await page.fill(SEL.username, `failure${Date.now()}`);
  await page.fill(SEL.password, "newpassword123");
  await click(page, SEL.submit);
  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page).toHaveURL(`${BASE_URL}/register`);
  await expect(page.locator(SEL.logoutLink)).toHaveCount(0);
});

test("register with open policy succeeds", async ({ page }) => {
  // Holdout (spec D6): registration::register coverage.
  const username = `newuser${Date.now()}${Math.random().toString(36).slice(2, 8)}`;
  await goto(page, "/register");

  await page.fill(SEL.username, username);
  await page.fill(SEL.password, "newpassword123");
  await click(page, SEL.submit);
  await waitForSelector(page, SEL.logoutLink);

  await expect(page.locator(SEL.error)).not.toBeVisible();
});

test("login page shows form", async ({ page }) => {
  // Holdout (spec D6): proves /login renders.
  await goto(page, "/login");

  await expect(page.locator("h1")).toHaveText("Login");
  await expect(page.locator(SEL.username)).toBeVisible();
  await expect(page.locator(SEL.password)).toBeVisible();
});

test("login with valid credentials succeeds", async ({
  page,
  user,
}, testInfo) => {
  // Holdout (spec D6): auth::login coverage.
  const perf = createPerfProbe(testInfo, "auth_login_success");

  await goto(page, "/login");

  await page.fill(SEL.username, user.username);
  await page.fill(SEL.password, user.password);
  perf.mark("credentials_filled");
  await click(page, SEL.submit);
  perf.mark("submit_clicked");
  // Login now redirects via client-side pushState (#591 dropped the full-reload
  // hook), so waitForURL is reliable — but we wait for the sidebar logout link,
  // which appears once the shared session context flips to authenticated, as the
  // content-readiness signal.
  await waitForSelector(page, SEL.logoutLink);
  perf.mark("logout_link_visible");

  // No waitForMount: login is a client-side pushState now, so `data-mounted`
  // (per-document) is already set — assert on content readiness instead (#591).
  await expect(page.locator(".j-sb-foot")).toContainText(user.username);
  await expect(page.locator(".j-sidebar")).toBeVisible();
  perf.mark("assertions_complete");
  await perf.log();
});

test("login submits with Enter", async ({ page, user }) => {
  await goto(page, "/login");
  await page.fill(SEL.username, user.username);
  await page.fill(SEL.password, user.password);
  await page.locator(SEL.password).press("Enter");
  await waitForSelector(page, SEL.logoutLink);
  await expect(page).toHaveURL(`${BASE_URL}/`);
});

test("login invalid fields do not dispatch", async ({ page }) => {
  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/auth/login")) requests += 1;
  });
  await goto(page, "/login");
  await page.fill(SEL.username, "invalid username");
  await page.locator(SEL.username).blur();
  await page.fill(SEL.password, "short");
  await page.locator(SEL.password).blur();

  await expect(page.locator(SEL.error)).toHaveCount(2);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await page.locator(SEL.password).press("Enter");
  expect(requests).toBe(0);
});

test("login pending state prevents duplicate dispatch", async ({
  page,
  user,
}) => {
  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/auth/login")) requests += 1;
  });
  await goto(page, "/login");
  const release = await stallServerFn(page, "auth/login");
  await page.fill(SEL.username, user.username);
  await page.fill(SEL.password, user.password);
  await click(page, SEL.submit);
  await expect.poll(() => requests).toBe(1);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await page.locator(SEL.password).press("Enter");
  expect(requests).toBe(1);
  release();
  await waitForSelector(page, SEL.logoutLink);
});

// #591: login/logout redirect via client-side pushState, so the wasm app is not
// re-booted. Proof: a value stashed on `window`
// before the action survives across it — a full document load would wipe it.
test("login navigates client-side without a full document reload", async ({
  page,
  user,
}) => {
  // Holdout (spec D6): login is a pushState, not a reload (#591).
  await goto(page, "/login");
  await page.evaluate(() => {
    (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload =
      true;
  });

  await page.fill(SEL.username, user.username);
  await page.fill(SEL.password, user.password);
  await click(page, SEL.submit);
  await waitForSelector(page, SEL.logoutLink);

  const survived = await page.evaluate(
    () =>
      (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload ===
      true,
  );
  expect(survived).toBe(true);
  await expect(page).toHaveURL(`${BASE_URL}/`);
});

test("logout navigates client-side without a full document reload", async ({
  page,
  user,
}) => {
  // Seeded session (login-as-setup); the logout itself is the subject.
  await signInAs(page, user.username);
  await goto(page, "/");
  await page.evaluate(() => {
    (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload =
      true;
  });

  await click(page, SEL.logoutLink);
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await expect(page.locator(".j-sb-foot")).not.toContainText(user.username);

  const survived = await page.evaluate(
    () =>
      (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload ===
      true,
  );
  expect(survived).toBe(true);
});

test("login with wrong password shows error", async ({ page }) => {
  // Holdout (spec D6): the login error path.
  await goto(page, "/login");

  await fillLoginForm(page, "testlogin", "wrongpassword!");
  await waitForSelector(page, SEL.error);

  await expect(page.locator(SEL.error)).toBeVisible();
});

test("logout page logs out", async ({ page, user }) => {
  // #649: /logout is a pure redirect trigger — leptos_router's redirect->pushState
  // navigates to "/" on the same resolution that would render a success message, so
  // there is no perceivable "You have been logged out." page. This test pins that the
  // flow ends signed-out at "/"; the LogoutPage render carries no success branch.
  // Seeded session (login-as-setup); the logout itself is the subject.
  await signInAs(page, user.username);
  await goto(page, "/");

  // Use the rendered logout link to avoid Firefox navigation abort races.
  await click(page, SEL.logoutLink);

  // Logout clears the session and redirects to "/" via client-side pushState
  // (#591); waitForURL is reliable for pushState navigations.
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  // Footer shows neither username nor sign-in link after logout.
  await expect(page.locator(".j-sb-foot")).not.toContainText(user.username);
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("sidebar reverts to signed-out state after logout", async ({
  page,
  user,
}) => {
  // Seeded session (login-as-setup); the logout itself is the subject.
  await signInAs(page, user.username);
  await goto(page, "/");
  // a[href='/logout'] only renders when auth Suspense resolves, confirming the
  // user is shown.
  await expect(page.locator(".j-sb-foot")).toContainText(user.username);

  // Click the sidebar "Sign out" link and confirm the sidebar switches back.
  await click(page, SEL.logoutLink);
  // Logout redirects to "/" via client-side pushState (#591); waitForURL is reliable.
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await expect(page.locator(".j-sb-foot")).not.toContainText(user.username);
  // The footer renders nothing when unauthenticated — no Sign-in link.
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
  registeredPage,
}) => {
  const page = await registeredPage("/");
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

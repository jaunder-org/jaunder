import { test, expect } from "./fixtures";
import { createPerfProbe } from "./perf";
import {
  BASE_URL,
  generateUsername,
  goto,
  click,
  waitForSelector,
  signInAs,
  fillLoginForm,
  failServerFn,
  stallServerFn,
} from "./helpers";
import { SEL } from "./selectors";
import { expectVisual } from "./visual";
import { expectAccessible } from "./accessibility";
import { navigateInApp } from "./navigate";
import { openComposerFromSidebar } from "./posts";

test("register page shows form", async ({ page }) => {
  // Holdout (spec D6): proves /register renders.
  await goto(page, "/register");

  await expect(page.locator("h1")).toHaveText("Register");
  await expect(page.locator(SEL.username)).toBeVisible();
  await expect(page.locator(SEL.password)).toBeVisible();
});

// #450 with-chrome proof: the registration form reaches ADR-0065 through
// `ValidatedInput<T>`, which wraps the same bare-input/error primitives as direct-bind
// sites while preserving disable-until-valid and touched-gated messages.
test("register invalid fields do not dispatch", async ({ page }) => {
  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/registration/register")) requests += 1;
  });
  await goto(page, "/register");

  await page.fill(SEL.username, "Bad User");
  await page.fill(SEL.password, "short");
  await page.locator(SEL.username).blur();
  await page.locator(SEL.password).blur();

  await expect(page.locator(SEL.error)).toHaveCount(2);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await page.locator(SEL.password).press("Enter");
  expect(requests).toBe(0);

  // Both valid values clear the errors and enable submit.
  await page.fill(SEL.username, "validusername");
  await page.fill(SEL.password, "longenough123");
  await expect(page.locator(SEL.error)).toHaveCount(0);
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
  const username = generateUsername("newuser");
  await goto(page, "/register");

  await page.fill(SEL.username, username);
  await page.fill(SEL.password, "newpassword123");
  await click(page, SEL.submit);
  await waitForSelector(page, SEL.logoutLink);
  await expect(page).toHaveURL(`${BASE_URL}/app`);

  await expect(page.locator(SEL.error)).not.toBeVisible();
});

test(
  "login page shows form",
  { tag: ["@visual", "@accessibility"] },
  async ({ page }) => {
    // Holdout (spec D6): proves /login renders.
    await goto(page, "/login");

    await expect(page.locator("h1")).toHaveText("Login");
    await expect(page.locator(SEL.username)).toBeVisible();
    await expect(page.locator(SEL.password)).toBeVisible();
    await expectVisual(page, "login-page.png");
    await expectAccessible(page);
  },
);

test("private routes withhold content while session reconciliation is pending", async ({
  page,
}) => {
  const release = await stallServerFn(page, "auth/get_session");
  await goto(page, "/sessions");

  await expect(page.locator(".j-loading")).toBeVisible();
  await expect(page.locator(".j-topbar h1")).toHaveCount(0);
  release();
  await page.waitForURL(`${BASE_URL}/login?return_to=%2Fsessions`);
});

test("anonymous private entry replaces history and remains in the SPA", async ({
  page,
}) => {
  const release = await stallServerFn(page, "auth/get_session");
  await goto(page, "/posts/42/history/7?order=oldest#revision");
  const historyLength = await page.evaluate(() => {
    (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload =
      true;
    return history.length;
  });
  release();

  await page.waitForURL(
    `${BASE_URL}/login?return_to=%2Fposts%2F42%2Fhistory%2F7%3Forder%3Doldest%23revision`,
  );
  await expect(page.locator(SEL.username)).toBeVisible();
  const survived = await page.evaluate(
    () =>
      (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload ===
      true,
  );
  expect(survived).toBe(true);
  await expect
    .poll(() => page.evaluate(() => history.length))
    .toBe(historyLength);
});

test("private session reconciliation failure retries without redirecting", async ({
  page,
}) => {
  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/auth/get_session")) requests += 1;
  });
  await failServerFn(page, "auth/get_session");
  await goto(page, "/sessions");

  await expect(page.locator(".error")).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
  await expect(page).toHaveURL(`${BASE_URL}/sessions`);
  const requestsBeforeRetry = requests;
  await click(page, 'button:has-text("Retry")');
  await expect.poll(() => requests).toBeGreaterThan(requestsBeforeRetry);
  await expect(page.locator(".error")).toBeVisible();
  await expect(page).toHaveURL(`${BASE_URL}/sessions`);
});

test("authenticated private routes mount member views during in-app navigation", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/sessions");
  await expect(page.locator(".j-topbar h1")).toHaveText("Sessions");

  await navigateInApp(page, () => page.click('.j-nav a[href="/profile"]'), {
    url: "/profile",
    ready: '.j-topbar h1:has-text("Profile")',
  });
});

test("authenticated operator mounts the operator-only private route", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/admin/backups");
  await expect(page.locator(".j-topbar h1")).toHaveText("Backup Settings");
});

test("authenticated non-operators remain on an unauthorized operator route", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/admin/backups");
  await expect(page.locator(".j-topbar h1")).toHaveText("Backup Settings");
  await expect(page.locator(".error")).toBeVisible();
  await expect(page).toHaveURL(`${BASE_URL}/admin/backups`);
});

test("authenticated users mount parameterized private routes", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/posts/42/history");
  await expect(page.locator(".j-topbar h1")).toHaveText("Post History");
});

test("direct login with valid credentials falls back to Home", async ({
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

  // Login's redirect is client-side pushState, so `data-mounted` (per-document)
  // is already set — assert directly on the Home destination.
  await expect(page).toHaveURL(`${BASE_URL}/app`);
  await expect(page.locator(".j-sb-foot")).toContainText(user.username);
  await expect(page.locator(".j-sidebar")).toBeVisible();
  await expect(page.locator(".j-nav a[href='/app']")).toHaveText("Home");
  await expect(page.locator('.j-nav a[href="/"]')).toHaveCount(0);
  perf.mark("assertions_complete");
  await perf.log();
});

test("confirmed password login returns to the exact private destination without a reload", async ({
  page,
  user,
}) => {
  const destination = "/posts/42/history/7?order=oldest#revision";
  await goto(page, `/login?return_to=${encodeURIComponent(destination)}`);
  await page.evaluate(() => {
    (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload =
      true;
  });

  await fillLoginForm(page, user.username, user.password);
  await waitForSelector(page, SEL.logoutLink);
  await expect(page).toHaveURL(`${BASE_URL}${destination}`);
  await expect(
    page.evaluate(
      () =>
        (window as Window & { __jaunderNoReload?: boolean })
          .__jaunderNoReload === true,
    ),
  ).resolves.toBe(true);
});

for (const returnTo of ["https://evil.example/app", "/login", "/unknown"]) {
  test(`invalid login return ${returnTo} falls back to Home`, async ({
    page,
    user,
  }) => {
    await goto(page, `/login?return_to=${encodeURIComponent(returnTo)}`);
    await fillLoginForm(page, user.username, user.password);
    await waitForSelector(page, SEL.logoutLink);
    await expect(page).toHaveURL(`${BASE_URL}/app`);
  });
}

test("login submits with Enter", async ({ page, user }) => {
  await goto(page, "/login");
  await page.fill(SEL.username, user.username);
  await page.fill(SEL.password, user.password);
  await page.locator(SEL.password).press("Enter");
  await waitForSelector(page, SEL.logoutLink);
  await expect(page).toHaveURL(`${BASE_URL}/app`);
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
  await expect(page).toHaveURL(`${BASE_URL}/app`);
});

test("logout navigates client-side without a full document reload", async ({
  page,
  user,
}) => {
  // Seeded session (login-as-setup); the logout itself is the subject.
  await signInAs(page, user.username);
  await goto(page, "/app");
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
  await goto(page, "/app");

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
  await goto(page, "/app");
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

test("sidebar shows Local only and no Compose link when not logged in", async ({
  page,
  firstNav,
}) => {
  await goto(page, "/", { timeout: firstNav });
  await waitForSelector(page, ".j-nav");
  const navAnchors = page.locator(".j-nav a");
  await expect(navAnchors).toHaveCount(1);
  await expect(navAnchors.first()).toHaveAttribute("href", "/");
  await expect(navAnchors.first()).toHaveText("Local");
  await expect(page.locator('.j-nav a[href="/app"]')).toHaveCount(0);
  await expect(page.locator('.j-nav a[href="/posts/new"]')).toHaveCount(0);
  await expect(page.locator(".j-sidebar")).not.toContainText("Sources");
  await expect(page.locator(".j-sidebar")).not.toContainText("Bluesky");
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("authenticated sidebar exposes Home and no Local", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/app");
  await waitForSelector(page, '.j-nav a[href="/posts/new"]');
  await waitForSelector(page, '.j-nav a[href="/drafts"]');
  await waitForSelector(page, '.j-nav a[href="/scheduled"]');
  // Home, Compose, Drafts, Scheduled, History, Media, Audiences, Themes,
  // Sessions, Passkeys, and Settings have hrefs.
  await waitForSelector(page, '.j-nav a[href="/audiences"]');
  await expect(page.locator(".j-sidebar")).not.toContainText("Sources");
  await expect(page.locator(".j-sidebar")).not.toContainText("Bluesky");
  await waitForSelector(page, '.j-nav a[href="/history"]');
  const navAnchors = page.locator(".j-nav a");
  await expect(navAnchors).toHaveCount(11);
  const navHrefs = await navAnchors.evaluateAll((links) =>
    links.map((link) => link.getAttribute("href")),
  );
  expect(navHrefs).toEqual([
    "/app",
    "/posts/new",
    "/drafts",
    "/scheduled",
    "/history",
    "/media",
    "/audiences",
    "/themes",
    "/sessions",
    "/passkeys",
    "/profile",
  ]);
  await expect(page.locator('.j-nav a[href="/app"]')).toHaveText("Home");
  await expect(page.locator('.j-nav a[href="/"]')).toHaveCount(0);
  await expect(page.locator('.j-nav a[href="/app"]')).toHaveClass(
    /\bis-active\b/,
  );
  await expect(page.locator(SEL.logoutLink)).toBeVisible();
  await expect(page.locator(".j-sb-foot a[href='/login']")).toHaveCount(0);
});

test("authenticated brand navigation targets Home directly", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/posts/new");
  await navigateInApp(page, () => page.click(".j-brand"), {
    url: "/app",
    ready: '.j-topbar h1:has-text("Home")',
  });
  await expect(page.locator(".j-nav a[href='/app']")).toHaveClass(
    /\bis-active\b/,
  );
});

test("sidebar active state follows exact in-app destinations", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/app");
  const activeItem = page.locator(".j-nav a.is-active");
  await waitForSelector(page, '.j-nav a[href="/posts/new"]');
  await expect(activeItem).toHaveCount(1);
  await expect(activeItem).toHaveAttribute("href", "/app");

  await openComposerFromSidebar(page);
  await expect(activeItem).toHaveCount(1);
  await expect(activeItem).toHaveAttribute("href", "/posts/new");

  await navigateInApp(page, () => page.click('.j-nav a[href="/drafts"]'), {
    url: "/drafts",
    ready: '.j-topbar h1:has-text("Drafts")',
  });
  await expect(activeItem).toHaveCount(1);
  await expect(activeItem).toHaveAttribute("href", "/drafts");
});

test("unmatched route leaves every sidebar item inactive", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/posts/999999999/edit");
  await waitForSelector(page, '.j-nav a[href="/posts/new"]');
  await expect(page.locator(SEL.error)).toContainText("Post not found");
  await expect(page.locator(".j-nav a.is-active")).toHaveCount(0);
});

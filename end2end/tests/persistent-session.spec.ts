import type { Request } from "@playwright/test";
import { test, expect } from "./fixtures";
import {
  BASE_URL,
  goto,
  fillLoginForm,
  waitForSelector,
  click,
} from "./helpers";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";
import { openArea } from "./settings-areas";

test("browser receives and uses a persistent Session cookie and logout clears it", async ({
  page,
  user,
  firstNav,
}) => {
  await goto(page, "/login", { timeout: firstNav });
  const loginResponse = page.waitForResponse((response) =>
    response.url().endsWith("/api/auth/login"),
  );
  const started = Math.floor(Date.now() / 1000);
  await fillLoginForm(page, user.username, user.password);
  await waitForSelector(page, SEL.accountControl);
  const response = await loginResponse;
  expect(response.ok()).toBe(true);
  const cookies = await page.context().cookies(BASE_URL);
  const session = cookies.find((cookie) => cookie.name === "session");
  expect(session).toBeDefined();
  expect(session!.expires).toBeGreaterThanOrEqual(started + 2_592_000 - 1);
  expect(session!.expires).toBeLessThanOrEqual(
    Math.floor(Date.now() / 1000) + 2_592_000,
  );
  expect(session!.httpOnly).toBe(true);
  expect(session!.sameSite).toBe("Lax");
  expect(session!.path).toBe("/");
  expect(session!.secure).toBe(new URL(BASE_URL).protocol === "https:");
  const setCookie = (await response.headersArray()).find(
    (header) =>
      header.name.toLowerCase() === "set-cookie" &&
      header.value.startsWith("session="),
  )?.value;
  expect(setCookie?.includes("Max-Age=")).toBe(true);
  expect(setCookie?.includes("Expires=")).toBe(true);
  expect(await page.evaluate(() => document.cookie.includes("session="))).toBe(
    false,
  );

  // A separate cold page uses the browser's received cookie, not a seeded
  // credential or copied token. This is not a universal restart/privacy-policy claim.
  const nextPage = await page.context().newPage();
  try {
    await goto(nextPage, "/sessions", { timeout: firstNav });
    await expect(nextPage.locator(".j-topbar h1")).toHaveText("Sessions");
    // The cookie-backed session is this user's: the Account page names them.
    await openArea(nextPage, "/account", ".j-area-identity");
    await expect(nextPage.locator(".j-area-identity")).toContainText(
      user.username,
    );
    // Sign-out ends in a tail of requests from `/`. Closing the page while one
    // is still being sent drops its traceparent, which orphans the server fn in
    // the coverage snapshot (#2003), so the page closes only once all settle.
    const pendingApi = new Set<Request>();
    let logouts = 0;
    nextPage.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (!path.startsWith("/api/")) return;
      pendingApi.add(request);
      if (path === "/api/auth/logout") logouts += 1;
    });
    const settle = (request: Request) => pendingApi.delete(request);
    nextPage.on("requestfinished", settle);
    nextPage.on("requestfailed", settle);
    await navigateInApp(nextPage, () => click(nextPage, SEL.signOutLink), {
      url: "/",
      ready: '.j-topbar h1:has-text("Jaunder")',
    });
    await expect.poll(() => pendingApi.size).toBe(0);
    // One sign-out is one logout, not a second dispatch after the redirect.
    expect(logouts).toBe(1);
    await expect
      .poll(async () =>
        (await page.context().cookies(BASE_URL)).some(
          (cookie) => cookie.name === "session",
        ),
      )
      .toBe(false);
  } finally {
    await nextPage.close();
  }
});

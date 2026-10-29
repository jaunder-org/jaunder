import { test, expect } from "./fixtures";
import {
  BASE_URL,
  goto,
  fillLoginForm,
  waitForSelector,
  click,
} from "./helpers";
import { SEL } from "./selectors";

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
  await waitForSelector(page, SEL.logoutLink);
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
    await expect(nextPage.locator(".j-sb-foot")).toContainText(user.username);
    await click(nextPage, SEL.logoutLink);
    await nextPage.waitForURL(`${BASE_URL}/`);
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

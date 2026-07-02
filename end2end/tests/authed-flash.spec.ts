/**
 * #181 (ADR-0044) — authenticated-owner flash-free enhancement.
 *
 * Asserts the pre-paint contract and the enhance-don't-replace behavior without
 * brittle pixel/CLS diffing (D8): the pre-paint script marks `html.authed`
 * before the WASM client's async work, `/` stays the enhanced public timeline
 * (never a personal-feed swap) with the owner's own-post affordance, and the
 * personalized feed lives at the bookmarkable `/app` cockpit (anon bounces to
 * `/login`). The strict empirical layout-shift assertion is the follow-up #202.
 */

import {
  test,
  expect,
  hydrationHeavyFirstNavigationTimeoutMs,
  hydrationHeavyTimeoutMs,
} from "./fixtures";
import type { Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { BASE_URL, goto, register } from "./helpers";

async function createPublishedPostViaApi(
  page: Page,
  title: string,
): Promise<void> {
  const response = await withTimedAction(page, "api.create_post", () =>
    page.request.post(`${BASE_URL}/api/create_post`, {
      data: {
        body: `# ${title}\n\nBody for ${title}`,
        format: "markdown",
        slug_override: null,
        publish: true,
      },
    }),
  );
  expect(response.ok()).toBeTruthy();
}

test("owner: pre-paint auth marks html.authed and / stays the enhanced public timeline", async ({
  page,
}, testInfo) => {
  const username = await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
  await createPublishedPostViaApi(page, "Owner Post");

  await goto(page, "/");

  // Pre-paint auth detection (D5): only the inline <head> script sets these — the
  // WASM client never does — so their presence proves auth was known pre-paint.
  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);

  // `/` stays the public Local timeline (D10) — NOT the personal "Your home feed".
  await expect(page.locator(".j-topbar h1")).toHaveText("jaunder.local");

  // The owner's own post gains the client-side action column (D4) — its Edit
  // affordance is absent from the anonymous seed data (is_author = false).
  await expect(
    page.locator('.j-post-acts a[href$="/edit"]').first(),
  ).toBeVisible({ timeout: hydrationHeavyTimeoutMs(testInfo, 10_000) });

  // Authed sidebar chrome is present (footer logout + an authed-only nav link).
  await expect(page.locator(".j-sb-foot a[href='/logout']")).toBeVisible();
  await expect(page.locator(".j-sidebar a[href='/drafts']")).toBeVisible();
});

test("owner: /app cockpit boots straight into the personalized feed", async ({
  page,
}, testInfo) => {
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Directly bookmarkable (D6): a direct hit to /app boots into the feed + composer
  // with zero intermediate clicks (pre-paint html.authed → the client boots authed).
  await goto(page, "/app");

  await expect(page.locator(".j-topbar .j-sub")).toHaveText("Your home feed");
  await expect(page.locator('textarea[name="body"]')).toBeVisible();
});

test("owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app", async ({
  page,
}, testInfo) => {
  // D7 / acceptance-#3: the redirect-pref read path exists in PREPAINT_SCRIPT with a
  // safe stay-default (nothing writes the key yet). Writing it exercises that path:
  // an authed owner (marker set) with the key = "app" is redirected off / to /app
  // before first paint. Requires BOTH the marker and the key.
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
  await page.evaluate(() =>
    localStorage.setItem("jaunder_home_redirect", "app"),
  );

  await page.goto(`${BASE_URL}/`, { waitUntil: "commit" });
  await page.waitForURL(/\/app$/, {
    timeout: hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  });
});

test("anonymous: /app bounces to /login", async ({ page }, testInfo) => {
  // No session and no marker → CockpitPage's current_user() gate resolves anon and
  // redirects to /login (D6).
  await page.goto(`${BASE_URL}/app`, { waitUntil: "domcontentloaded" });
  await page.waitForURL(/\/login$/, {
    timeout: hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  });
});

test("anonymous: / has no authed sidebar chrome", async ({ page }) => {
  await goto(page, "/");

  await expect(page.locator("html")).not.toHaveClass(/\bauthed\b/);
  await expect(page.locator("a[href='/logout']")).toHaveCount(0);
  await expect(page.locator(".j-sidebar a[href='/drafts']")).toHaveCount(0);
});

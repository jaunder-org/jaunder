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

import { test, expect, slowBrowserTimeoutMs } from "./fixtures";
import { BASE_URL, goto, register } from "./helpers";
import { SEL } from "./selectors";
import { createPostViaApi } from "./posts";

test("owner: pre-paint auth marks html.authed and / stays the enhanced public timeline", async ({
  page,
  firstNav,
}, testInfo) => {
  const username = await register(page, firstNav);
  await createPostViaApi(page, { body: "# Owner Post\n\nBody for Owner Post" });

  await goto(page, "/");

  // Pre-paint auth detection (D5): only the inline <head> script sets these — the
  // WASM client never does — so their presence proves auth was known pre-paint.
  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);

  // `/` stays the public Local timeline (D10) — NOT the personal "Your home feed".
  await expect(page.locator(SEL.topbarHeading)).toHaveText("jaunder.local");

  // #319: the anon Sign-in/Register CTA is server-painted but `j-anon-only`, so
  // the pre-paint `html.authed` hides it for the owner (no flash). Use CSS
  // locators (which match hidden nodes) so this asserts present-but-hidden, not
  // merely absent — `getByRole` skips `display:none` elements and would pass
  // vacuously.
  await expect(page.locator('main a[href="/login"]')).toBeHidden();
  await expect(page.locator('main a[href="/register"]')).toBeHidden();

  // The owner's own post gains the client-side action column (D4) — its Edit
  // affordance is absent from the anonymous seed data (is_author = false).
  await expect(
    page.locator('.j-post-acts a[href$="/edit"]').first(),
  ).toBeVisible({ timeout: slowBrowserTimeoutMs(testInfo, 10_000) });

  // Authed sidebar chrome is present (footer logout + an authed-only nav link).
  await expect(page.locator(".j-sb-foot a[href='/logout']")).toBeVisible();
  await expect(page.locator(".j-sidebar a[href='/drafts']")).toBeVisible();
});

test("owner: /app cockpit boots straight into the personalized feed", async ({
  registeredPage: page,
}) => {
  // Directly bookmarkable (D6): a direct hit to /app boots into the feed + composer
  // with zero intermediate clicks (pre-paint html.authed → the client boots authed).
  await goto(page, "/app");

  await expect(page.locator(".j-topbar .j-sub")).toHaveText("Your home feed");
  await expect(page.locator(SEL.postBody)).toBeVisible();
});

test("owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app", async ({
  page,
  firstNav,
}) => {
  // D7 / acceptance-#3: the redirect-pref read path exists in PREPAINT_SCRIPT with a
  // safe stay-default (nothing writes the key yet). Writing it exercises that path:
  // an authed owner (marker set) with the key = "app" is redirected off / to /app
  // before first paint. Requires BOTH the marker and the key.
  await register(page, firstNav);
  await page.evaluate(() =>
    localStorage.setItem("jaunder_home_redirect", "app"),
  );

  await page.goto(`${BASE_URL}/`, { waitUntil: "commit" });
  await page.waitForURL(/\/app$/, {
    timeout: firstNav,
  });
});

test("anonymous: /app bounces to /login", async ({ page, firstNav }) => {
  // No session and no marker → CockpitPage's current_user() gate resolves anon and
  // redirects to /login (D6).
  await page.goto(`${BASE_URL}/app`, { waitUntil: "domcontentloaded" });
  await page.waitForURL(/\/login$/, {
    timeout: firstNav,
  });
});

test("anonymous: / has no authed sidebar chrome", async ({ page }) => {
  await goto(page, "/");

  await expect(page.locator("html")).not.toHaveClass(/\bauthed\b/);
  await expect(page.locator(SEL.logoutLink)).toHaveCount(0);
  await expect(page.locator(".j-sidebar a[href='/drafts']")).toHaveCount(0);
});

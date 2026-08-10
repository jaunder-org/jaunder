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
import {
  BASE_URL,
  click,
  goto,
  login,
  registerViaUi,
  signInAs,
  signInAsNewUser,
  failServerFn,
} from "./helpers";
import { SEL } from "./selectors";
import { createPostViaApi } from "./posts";

test("owner: pre-paint auth marks html.authed and / stays the enhanced public timeline", async ({
  page,
  firstNav,
}, testInfo) => {
  // Holdout (spec D6): registering through the real UI leaves a correct marker.
  const username = await registerViaUi(page, firstNav);
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

// AC5 (#791): a seeded session — no UI flow — must satisfy the same pre-paint
// contract as the registerViaUi holdout above. This is what proves D3's
// tombstoned init script feeds the <head> script.
test("seeded: pre-paint auth marks html.authed and data-user", async ({
  page,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  await goto(page, "/", { timeout: firstNav });

  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);
});

// D3 (#791): after a UI logout the init script must NOT re-apply the seeded
// marker — the tombstone (applied == companion cookie) makes it a no-op. The
// pushState logout tests never re-run an init script, so only a full
// post-logout navigation pins this.
test("seeded: logout survives a full navigation (tombstone respected)", async ({
  page,
  firstNav,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/", { timeout: firstNav });
  await click(page, SEL.logoutLink);
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });

  await goto(page, "/", { timeout: firstNav });

  await expect(page.locator("html")).not.toHaveClass(/\bauthed\b/);
  await expect(page.locator(SEL.logoutLink)).toHaveCount(0);
});

// D3 (#791): the nonce row — seed → logout → re-seed the SAME user. The new
// seed's companion value differs (fresh nonce), so the init script re-applies
// the marker and the page boots authed again pre-paint.
test("seeded: re-seed as the same user after logout boots authed", async ({
  page,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  await goto(page, "/", { timeout: firstNav });
  await click(page, SEL.logoutLink);
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });

  await signInAs(page, username);
  await goto(page, "/", { timeout: firstNav });

  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);
});

test("owner: /app cockpit boots straight into the personalized feed", async ({
  registeredPage,
}) => {
  // Directly bookmarkable (D6): a direct hit to /app boots into the feed + composer
  // with zero intermediate clicks (pre-paint html.authed → the client boots authed).
  const page = await registeredPage("/app");

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
  // Holdout (spec D6): the pre-paint redirect path, on a real UI-written marker
  // (a seeded helper does not navigate, so the localStorage write below would
  // land on about:blank).
  await registerViaUi(page, firstNav);
  await page.evaluate(() =>
    localStorage.setItem("jaunder_home_redirect", "app"),
  );

  await page.goto(`${BASE_URL}/`, { waitUntil: "commit" });
  await page.waitForURL(/\/app$/, {
    timeout: firstNav,
  });
});

test("anonymous: /app bounces to /login", async ({ page, firstNav }) => {
  // No session and no marker → CockpitPage's session-reconcile gate resolves anon
  // and redirects to /login (D6).
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

// #591: operator status now rides in the auth marker, so operator chrome is seeded
// flash-free on boot (not awaited from a server fetch). Proof: fail the `get_session()`
// reconcile so no server confirmation can arrive — the operator admin nav must still
// paint, sourced from the marker seed alone.
test("operator: admin chrome is seeded flash-free from the marker", async ({
  page,
}) => {
  // Log in as the seeded operator; this writes the marker with is_operator:true.
  // Holdout (spec D6): logging in through the real UI leaves a correct marker.
  await login(page, "testoperator", "testpassword123");

  // With get_session() failing, the operator admin nav can only come from the marker.
  await failServerFn(page, "auth/get_session");
  await goto(page, "/");

  await expect(
    page.locator(".j-sidebar a[href='/admin/backups']"),
  ).toBeVisible();
});

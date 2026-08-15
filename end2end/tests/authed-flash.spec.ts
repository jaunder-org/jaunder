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
import { allowEngineDependentBoot, allowSecondBoot } from "./bootBudget";
import { SEL } from "./selectors";
import { createPostViaApi } from "./posts";
import { applySeededSession, createSessionViaTool } from "./seed";
import { expectVisual } from "./visual";
import { expectAccessible } from "./accessibility";

test("owner: pre-paint auth marks html.authed and / stays the enhanced public timeline", async ({
  page,
  firstNav,
}, testInfo) => {
  // Holdout (spec D6): registering through the real UI leaves a correct marker.
  const username = await registerViaUi(page, firstNav);
  await createPostViaApi(page, { body: "# Owner Post\n\nBody for Owner Post" });

  allowSecondBoot(
    page,
    "the pre-paint `html.authed` marking is observable only on a cold document load, and it is the subject",
  );
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

  allowSecondBoot(
    page,
    "a full post-logout document load is exactly what pins the tombstone; the pushState logout tests never re-run the init script",
  );
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
  allowSecondBoot(
    page,
    "the re-seeded marker is re-applied by the init script only on a fresh document load, and booting authed again is the subject",
  );
  await goto(page, "/", { timeout: firstNav });

  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);
});

test(
  "owner: /app cockpit boots straight into the personalized feed",
  { tag: ["@visual", "@accessibility"] },
  async ({ page, firstNav }) => {
    const session = await createSessionViaTool("testlogin");
    await applySeededSession(page.context(), session);
    await goto(page, "/app", { timeout: firstNav });

    await expect(page.locator(".j-topbar .j-sub")).toHaveText("Your home feed");
    await expect(page.locator(SEL.postBody)).toBeVisible();
    await expectVisual(page, "authenticated-cockpit.png");
    await expectAccessible(page);
  },
);

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

  // How many further loads this produces is ENGINE-DEPENDENT, so it takes both
  // declaration forms. The `/` document commits and the pre-paint
  // `location.replace("/app")` fires during head parsing. On chromium `/` is
  // replaced before it ever reaches DOMContentLoaded, so the budget counts one
  // load and its URL is `/app`; on firefox `/` does fire the event, so the budget
  // counts two. (Measured on both — `framenavigated` fires for `/` and `/app`
  // everywhere; `domcontentloaded` for `/app` everywhere and for `/` on firefox
  // only.) `/app` always lands, so it is declared exactly; `/` is declared with
  // the engine-dependent form. Neither can be routed in-app: the redirect is the
  // script's own, and it is the subject.
  allowSecondBoot(
    page,
    "the /app load the pre-paint location.replace always produces; it is the script's own redirect, not a test-issued navigation, and it is the subject",
  );
  allowEngineDependentBoot(
    page,
    "/",
    "the / document itself: the pre-paint location.replace runs during head parsing, so whether / reaches DOMContentLoaded before being replaced is engine timing — firefox fires it and counts the load, chromium replaces / first and never does",
  );
  // e2e-goto-wrapper:allow `waitUntil: "commit"` plus the `waitForURL` below is the subject — the pre-paint redirect replaces `/` during head parsing, so the wrapper would wait for a mount on a document that never paints
  await page.goto(`${BASE_URL}/`, { waitUntil: "commit" });
  await page.waitForURL(/\/app$/, {
    timeout: firstNav,
  });
});

test("anonymous: /app bounces to /login", async ({ page, firstNav }) => {
  // No session and no marker → CockpitPage's session-reconcile gate resolves anon
  // and redirects to /login (D6).
  // e2e-goto-wrapper:allow the subject is the bounce itself, so this waits on the URL and not on the mount — the wrapper would insert a mount barrier on the /app document before the redirect is ever observed
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
  allowSecondBoot(
    page,
    "the pre-paint marker read happens only on a cold boot, and with get_session() failing that boot is the only source of the operator chrome",
  );
  await goto(page, "/");

  await expect(
    page.locator(".j-sidebar a[href='/admin/backups']"),
  ).toBeVisible();
});

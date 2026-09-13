/**
 * #181 (ADR-0044) — authenticated-owner flash-free enhancement.
 *
 * Asserts the pre-paint contract without brittle pixel/CLS diffing (D8): the
 * blocking script redirects a valid marker from Local to Home before Local can
 * paint, while `/app` remains a separately session-confirmed cockpit. The strict
 * empirical layout-shift assertion is the follow-up #202.
 */

import type { BrowserContext } from "@playwright/test";

import { test, expect } from "./fixtures";
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
import { applySeededSession, createSessionViaTool } from "./seed";
import { expectVisual } from "./visual";
import { expectAccessible } from "./accessibility";

test("valid marker: root redirects to Home before Local paints", async ({
  page,
  firstNav,
}) => {
  const username = await registerViaUi(page, firstNav);
  allowSecondBoot(
    page,
    "the pre-paint redirect loads Home after the authenticated transition has already mounted",
  );
  allowEngineDependentBoot(
    page,
    "/",
    "the root document can commit before its blocking head script replaces it, depending on engine timing",
  );
  // e2e-goto-wrapper:allow the blocking root redirect is the behavior under test.
  await page.goto(`${BASE_URL}/`, { waitUntil: "commit" });
  await page.waitForURL(/\/app$/, { timeout: firstNav });
  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);
  await expect(page.locator(".j-nav a[href='/app']")).toHaveText("Home");
  await expect(page.locator('.j-nav a[href="/"]')).toHaveCount(0);
});

// D3 (#791): after a UI logout the init script must NOT re-apply the seeded
// marker — its matching tombstone makes it a no-op. The pushState logout tests
// never re-run an init script, so only a full post-logout navigation pins this.
test("seeded: logout survives a full navigation (tombstone respected)", async ({
  page,
  firstNav,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/app", { timeout: firstNav });
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

// D3 (#791): seed → logout → re-seed the SAME user. The fresh nonce makes the
// replacement init script re-apply the marker and boot authenticated again.
test("seeded: re-seed as the same user after logout boots authed", async ({
  page,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  await goto(page, "/app", { timeout: firstNav });
  await click(page, SEL.logoutLink);
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });

  await signInAs(page, username);
  allowSecondBoot(
    page,
    "the re-seeded marker is re-applied by the init script only on a fresh document load, and booting authed again is the subject",
  );
  await goto(page, "/app", { timeout: firstNav });

  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);
});

test("seeded helper: replacing a session disposes before installing and injecting", async () => {
  const events: string[] = [];
  const live = new Set<number>();
  let nextHandle = 0;
  const context = {
    async addInitScript(script: string) {
      events.push(`install:${script}`);
      const handle = ++nextHandle;
      live.add(handle);
      return {
        async dispose() {
          events.push(`dispose:${handle}`);
          live.delete(handle);
        },
      };
    },
    async addCookies() {
      events.push("cookie");
    },
  } as unknown as BrowserContext;
  const session = {
    setCookie: "session=token; Path=/; HttpOnly; SameSite=Lax",
    markerKey: "marker-key",
    marker: "marker-value",
  };

  await applySeededSession(context, session);
  await applySeededSession(context, session);

  expect(events.map((event) => event.split(":")[0])).toEqual([
    "install",
    "cookie",
    "dispose",
    "install",
    "cookie",
  ]);
  expect(live).toEqual(new Set([2]));
  expect(events[3]).toContain("marker-key");
  expect(events[3]).toContain("marker-value");
  expect(events[3]).not.toContain("document.cookie");
});

test("seeded helper: a disposal failure retains its handle for retry", async () => {
  let disposeFails = false;
  let disposeCalls = 0;
  let installs = 0;
  const context = {
    async addInitScript() {
      ++installs;
      return {
        async dispose() {
          ++disposeCalls;
          if (disposeFails) throw new Error("dispose failed");
        },
      };
    },
    async addCookies() {},
  } as unknown as BrowserContext;
  const session = {
    setCookie: "session=token; Path=/; HttpOnly; SameSite=Lax",
    markerKey: "marker-key",
    marker: "marker-value",
  };

  await applySeededSession(context, session);
  disposeFails = true;
  await expect(applySeededSession(context, session)).rejects.toThrow(
    "dispose failed",
  );
  disposeFails = false;
  await applySeededSession(context, session);

  expect(disposeCalls).toBe(2);
  expect(installs).toBe(2);
});

test("seeded helper: an installation failure leaves the context untracked", async () => {
  let installFails = false;
  let disposeCalls = 0;
  let installs = 0;
  const context = {
    async addInitScript() {
      ++installs;
      if (installFails) throw new Error("install failed");
      return {
        async dispose() {
          ++disposeCalls;
        },
      };
    },
    async addCookies() {},
  } as unknown as BrowserContext;
  const session = {
    setCookie: "session=token; Path=/; HttpOnly; SameSite=Lax",
    markerKey: "marker-key",
    marker: "marker-value",
  };

  await applySeededSession(context, session);
  installFails = true;
  await expect(applySeededSession(context, session)).rejects.toThrow(
    "install failed",
  );
  installFails = false;
  await applySeededSession(context, session);

  expect(disposeCalls).toBe(1);
  expect(installs).toBe(3);
});

test("seeded helper: a cookie failure retains the replacement for retry", async () => {
  let cookieFails = false;
  let installs = 0;
  const disposed: number[] = [];
  const context = {
    async addInitScript() {
      const handle = ++installs;
      return {
        async dispose() {
          disposed.push(handle);
        },
      };
    },
    async addCookies() {
      if (cookieFails) throw new Error("cookie failed");
    },
  } as unknown as BrowserContext;
  const session = {
    setCookie: "session=token; Path=/; HttpOnly; SameSite=Lax",
    markerKey: "marker-key",
    marker: "marker-value",
  };

  await applySeededSession(context, session);
  cookieFails = true;
  await expect(applySeededSession(context, session)).rejects.toThrow(
    "cookie failed",
  );
  cookieFails = false;
  await applySeededSession(context, session);

  expect(disposed).toEqual([1, 2]);
  expect(installs).toBe(3);
});

test("seeded: re-seeding replaces the pre-paint identity on existing and new pages", async ({
  page,
  firstNav,
}) => {
  const firstUser = await signInAsNewUser(page);
  await goto(page, "/app", { timeout: firstNav });
  await expect(page.locator("html")).toHaveAttribute("data-user", firstUser);

  const replacement = await createSessionViaTool("testlogin");
  await applySeededSession(page.context(), replacement);
  allowSecondBoot(
    page,
    "the replacement seeded identity is observable only after a later document load",
  );
  await goto(page, "/app");
  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute(
    "data-user",
    replacement.username,
  );

  const newPage = await page.context().newPage();
  try {
    await goto(newPage, "/app", { timeout: firstNav });
    await expect(newPage.locator("html")).toHaveClass(/\bauthed\b/);
    await expect(newPage.locator("html")).toHaveAttribute(
      "data-user",
      replacement.username,
    );
  } finally {
    await newPage.close();
  }
});

test(
  "owner: /app cockpit boots straight into Home",
  { tag: ["@visual", "@accessibility"] },
  async ({ page, firstNav }) => {
    const session = await createSessionViaTool("testlogin");
    await applySeededSession(page.context(), session);
    await goto(page, "/app", { timeout: firstNav });
    await expect(page.locator(".j-topbar .j-sub")).toHaveText(
      "Your published Posts",
    );
    await expect(page.locator(SEL.postBody)).toBeVisible();
    await expectVisual(page, "authenticated-cockpit.png");
    await expectAccessible(page);
  },
);

test("recognized oldest order is the only root query state carried to Home", async ({
  page,
  firstNav,
}) => {
  await signInAsNewUser(page);
  allowEngineDependentBoot(
    page,
    "/",
    "the root document can commit before the blocking head script replaces it",
  );
  // e2e-goto-wrapper:allow the blocking root redirect canonicalization is the behavior under test.
  await page.goto(`${BASE_URL}/?order=oldest`, { waitUntil: "commit" });
  await page.waitForURL(/\/app\?order=oldest$/, { timeout: firstNav });
});

test("unrecognized root order redirects to canonical Home", async ({
  page,
  firstNav,
}) => {
  await signInAsNewUser(page);
  allowEngineDependentBoot(
    page,
    "/",
    "the root document can commit before the blocking head script replaces it",
  );
  // e2e-goto-wrapper:allow the blocking root redirect canonicalization is the behavior under test.
  await page.goto(`${BASE_URL}/?order=invalid&ignored=value`, {
    waitUntil: "commit",
  });
  await page.waitForURL(/\/app$/, { timeout: firstNav });
});

test("malformed marker leaves Local in place", async ({ page, firstNav }) => {
  await page.addInitScript(() => {
    localStorage.setItem("jaunder_auth", "{not json");
  });
  await goto(page, "/", { timeout: firstNav });
  await expect(page).toHaveURL(`${BASE_URL}/`);
  await expect(page.locator(".j-nav a[href='/']")).toHaveText("Local");
  await expect(page.locator('.j-nav a[href="/app"]')).toHaveCount(0);
});

test("marker missing its username leaves Local in place", async ({
  page,
  firstNav,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("jaunder_auth", JSON.stringify({}));
  });
  await goto(page, "/", { timeout: firstNav });
  await expect(page).toHaveURL(`${BASE_URL}/`);
  await expect(page.locator(".j-nav a[href='/']")).toHaveText("Local");
});

test("invalid marker username leaves Local in place", async ({
  page,
  firstNav,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "jaunder_auth",
      JSON.stringify({ username: "not valid" }),
    );
  });
  await goto(page, "/", { timeout: firstNav });
  await expect(page).toHaveURL(`${BASE_URL}/`);
  await expect(page.locator(".j-nav a[href='/']")).toHaveText("Local");
});

test("marker with invalid operator state leaves Local in place", async ({
  page,
  firstNav,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "jaunder_auth",
      JSON.stringify({ username: "alice", is_operator: "not-a-boolean" }),
    );
  });
  await goto(page, "/", { timeout: firstNav });
  await expect(page).toHaveURL(`${BASE_URL}/`);
  await expect(page.locator(".j-nav a[href='/']")).toHaveText("Local");
});

test("stale marker reaches login through Home without a redirect loop", async ({
  page,
  firstNav,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      "jaunder_auth",
      JSON.stringify({ username: "stale-user" }),
    );
  });
  allowEngineDependentBoot(
    page,
    "/",
    "the root document can commit before the blocking stale-marker redirect",
  );
  // e2e-goto-wrapper:allow the stale-marker redirect chain is the behavior under test.
  await page.goto(`${BASE_URL}/`, { waitUntil: "commit" });
  await page.waitForURL(/\/login$/, { timeout: firstNav });
  await expect(page.locator(SEL.postBody)).toHaveCount(0);
});

test("live session missing its marker reconciles Local to Home", async ({
  page,
  firstNav,
}) => {
  await registerViaUi(page, firstNav);
  await page.evaluate(() => {
    localStorage.removeItem("jaunder_auth");
  });
  allowSecondBoot(
    page,
    "the live session reaches a fresh markerless Local document before reconciliation",
  );
  await goto(page, "/", { timeout: firstNav });
  await page.waitForURL(`${BASE_URL}/app`, { timeout: firstNav });
});

test("missing marker reconciliation preserves recognized oldest order", async ({
  page,
  firstNav,
}) => {
  await registerViaUi(page, firstNav);
  await page.evaluate(() => {
    localStorage.removeItem("jaunder_auth");
  });
  allowSecondBoot(
    page,
    "the live session reaches a fresh markerless Local document before reconciliation",
  );
  await goto(page, "/?order=oldest", { timeout: firstNav });
  await page.waitForURL(`${BASE_URL}/app?order=oldest`, { timeout: firstNav });
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
  await goto(page, "/app");

  await expect(
    page.locator(".j-sidebar a[href='/admin/backups']"),
  ).toBeVisible();
});

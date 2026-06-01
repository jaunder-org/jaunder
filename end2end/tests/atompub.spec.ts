import { test, expect } from "@playwright/test";
import { goto, register, click } from "./helpers";
import {
  hydrationHeavyTimeoutMs,
  hydrationHeavyFirstNavigationTimeoutMs,
} from "./fixtures";

test("RSD autodiscovery link is present on the user page and resolves", async ({
  page,
}, info) => {
  info.setTimeout(hydrationHeavyTimeoutMs(info, 60_000));
  const username = await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(info, 30_000),
  );

  // The canonical user URL is ~-prefixed.
  await goto(page, `/~${username}`);

  const editUri = await page.$$eval(
    'head link[rel="EditURI"]',
    (els) =>
      els.map((e) => ({
        href: (e as HTMLLinkElement).href,
        type: (e as HTMLLinkElement).type,
      }))[0] ?? null,
  );

  expect(editUri, "EditURI link on user page").toBeTruthy();
  expect(editUri!.type).toBe("application/rsd+xml");
  expect(editUri!.href).toContain(`/~${username}/rsd.xml`);

  // The RSD document resolves and advertises the AtomPub service endpoint.
  const res = await page.request.get(editUri!.href);
  expect(res.status()).toBe(200);
  expect(res.headers()["content-type"]).toContain("application/rsd+xml");
  const body = await res.text();
  expect(body).toContain("<engineName>Jaunder</engineName>");
  expect(body).toContain("/atompub/service");
});

test("an app password can be minted from the sessions page", async ({
  page,
}, info) => {
  info.setTimeout(hydrationHeavyTimeoutMs(info, 60_000));
  await register(page, hydrationHeavyFirstNavigationTimeoutMs(info, 30_000));

  await goto(page, "/sessions");

  // goto waits for hydration, so the label input is safe to fill.
  await page.fill('input[name="label"]', "MarsEdit e2e");
  await click(page, '.j-app-passwords button[type="submit"]');

  // The raw token is shown exactly once.
  const tokenEl = page.locator(".j-app-password-token code");
  await tokenEl.waitFor({ state: "visible", timeout: 15_000 });
  const token = ((await tokenEl.textContent()) ?? "").trim();
  expect(token.length).toBeGreaterThan(10);

  // The new app password appears in the session list under its label.
  await expect(page.locator("li", { hasText: "MarsEdit e2e" })).toBeVisible();
});

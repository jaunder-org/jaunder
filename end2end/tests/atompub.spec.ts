import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { goto, register, click, BASE_URL } from "./helpers";
import {
  slowBrowserTimeoutMs,
  slowBrowserFirstNavigationTimeoutMs,
} from "./fixtures";

/// Mints an app password via the Sessions UI and returns the raw token.
async function mintAppPassword(page: Page, label: string): Promise<string> {
  await goto(page, "/sessions");
  await page.fill('input[name="label"]', label);
  await click(page, '.j-app-passwords button[type="submit"]');
  const tokenEl = page.locator(".j-app-password-token code");
  await tokenEl.waitFor({ state: "visible", timeout: 15_000 });
  return ((await tokenEl.textContent()) ?? "").trim();
}

/// A tiny valid 1x1 PNG.
const PNG = Buffer.from([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49,
  0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
  0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44,
  0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d,
  0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
  0x60, 0x82,
]);

/// Re-bases a URL's path onto the live test server. AtomPub emits absolute URLs
/// using the configured site base URL (`https://example.com` in the e2e VM),
/// which is not the address the test server actually listens on.
function onServer(url: string): string {
  try {
    const u = new URL(url);
    return `${BASE_URL}${u.pathname}${u.search}`;
  } catch {
    return `${BASE_URL}${url}`;
  }
}

test("RSD autodiscovery link is present on the user page and resolves", async ({
  page,
}, info) => {
  info.setTimeout(slowBrowserTimeoutMs(info, 60_000));
  const username = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
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
  info.setTimeout(slowBrowserTimeoutMs(info, 60_000));
  await register(page, slowBrowserFirstNavigationTimeoutMs(info, 30_000));

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

test("full AtomPub publishing flow over HTTP with an app password", async ({
  page,
  request,
}, info) => {
  info.setTimeout(slowBrowserTimeoutMs(info, 90_000));
  const username = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
  );

  const token = await mintAppPassword(page, "AtomPub e2e");
  // The `request` fixture carries no browser cookies, so these calls exercise
  // the app-password HTTP Basic auth path rather than the session cookie.
  const auth =
    "Basic " + Buffer.from(`${username}:${token}`).toString("base64");
  const xml = { authorization: auth, "content-type": "application/atom+xml" };

  // 1. Service document.
  const service = await request.get(`${BASE_URL}/atompub/service`, {
    headers: { authorization: auth },
  });
  expect(service.status()).toBe(200);
  expect(await service.text()).toContain("app:service");

  // 2. Create a post.
  const created = await request.post(`${BASE_URL}/atompub/${username}/posts`, {
    headers: xml,
    data: `<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>E2E Post</title>
  <content type="html">&lt;p&gt;hello from e2e&lt;/p&gt;</content>
  <category term="e2e"/>
</entry>`,
  });
  expect(created.status()).toBe(201);
  const memberUrl = onServer(created.headers()["location"]);
  expect(memberUrl).toContain(`/atompub/${username}/posts/`);

  // 3. Fetch the member entry (native-source HTML form, with the category).
  const member = await request.get(memberUrl, {
    headers: { authorization: auth },
  });
  expect(member.status()).toBe(200);
  const memberBody = await member.text();
  expect(memberBody).toContain('type="html"');
  expect(memberBody).toContain("hello from e2e");
  expect(memberBody).toContain('term="e2e"');

  // 4. List the collection feed.
  const list = await request.get(`${BASE_URL}/atompub/${username}/posts`, {
    headers: { authorization: auth },
  });
  expect(list.status()).toBe(200);
  const listBody = await list.text();
  expect(listBody).toContain("<feed");
  expect(listBody).toContain('rel="edit"');

  // 5. Edit the post.
  const edited = await request.put(memberUrl, {
    headers: xml,
    data: `<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>E2E Post edited</title>
  <content type="html">&lt;p&gt;edited body&lt;/p&gt;</content>
</entry>`,
  });
  expect(edited.status()).toBe(200);
  expect(await edited.text()).toContain("edited body");

  // 6. Upload media (raw bytes + Slug).
  const media = await request.post(`${BASE_URL}/atompub/${username}/media`, {
    headers: {
      authorization: auth,
      "content-type": "image/png",
      slug: "e2e.png",
    },
    data: PNG,
  });
  expect(media.status()).toBe(201);
  const mediaBody = await media.text();
  expect(mediaBody).toContain('rel="edit-media"');
  expect(mediaBody).toContain("/media/upload/");

  // 7. Delete the post; it is then gone.
  const del = await request.delete(memberUrl, {
    headers: { authorization: auth },
  });
  expect(del.status()).toBe(204);
  const gone = await request.get(memberUrl, {
    headers: { authorization: auth },
  });
  expect(gone.status()).toBe(404);
});

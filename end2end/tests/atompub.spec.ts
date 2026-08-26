import type { Page } from "@playwright/test";
import { goto, signInAsNewUser, click, BASE_URL } from "./helpers";
// `test` comes from the shared fixtures, not @playwright/test, so this spec emits
// an `e2e.test` span and its server-fn traffic (app-password minting, AtomPub
// publishing over HTTP) is attributable to a named test (#681).
import { test, expect } from "./fixtures";

/// Mints an app password via the Sessions UI and returns the raw token.
async function mintAppPassword(page: Page, label: string): Promise<string> {
  await goto(page, "/sessions");
  await page.fill("#app-password-label", label);
  await click(page, '.j-app-passwords button:has-text("Create app password")');
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
}) => {
  const username = await signInAsNewUser(page);

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
}) => {
  await signInAsNewUser(page);

  await goto(page, "/sessions");

  // goto waits for the CSR mount, so the label input is safe to fill.
  await page.fill("#app-password-label", "MarsEdit e2e");
  await click(page, '.j-app-passwords button:has-text("Create app password")');

  // The raw token is shown exactly once.
  const tokenEl = page.locator(".j-app-password-token code");
  await tokenEl.waitFor({ state: "visible", timeout: 15_000 });
  const token = ((await tokenEl.textContent()) ?? "").trim();
  expect(token.length).toBeGreaterThan(10);

  // The new app password appears in the session list under its label.
  await expect(page.locator("li", { hasText: "MarsEdit e2e" })).toBeVisible();
});

test("an app password can be revoked from the sessions page", async ({
  page,
  request,
}) => {
  const username = await signInAsNewUser(page);
  const label = "Revoked App Password e2e";

  const token = await mintAppPassword(page, label);
  const appPasswordRow = page.locator("li", { hasText: label });
  await expect(appPasswordRow).toBeVisible();

  await click(page, `li:has-text("${label}") button:has-text("Revoke")`);

  await expect(appPasswordRow).toHaveCount(0);
  await expect(page.locator("li", { hasText: "(current)" })).toBeVisible();

  const auth =
    "Basic " + Buffer.from(`${username}:${token}`).toString("base64");
  const response = await request.get(`${BASE_URL}/atompub/service`, {
    headers: { authorization: auth },
  });
  expect(response.ok()).toBeFalsy();
  expect(response.status()).toBe(401);
});

test("full AtomPub Org publishing flow over HTTP with an app password", async ({
  page,
  request,
}) => {
  const username = await signInAsNewUser(page);

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

  // 2. Create an Org post. Atom title wins over the header, while omitted
  // categories and summary are supplied by its Org metadata.
  const created = await request.post(`${BASE_URL}/atompub/${username}/posts`, {
    headers: xml,
    data: `<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Atom title</title>
  <content type="text/org">#+TITLE: Header title
#+KEYWORDS: header-category
#+DESCRIPTION: Header summary
#+PROPERTY: JAUNDER_STATUS draft
#+UNKNOWN: preserved

Org body</content>
</entry>`,
  });
  expect(created.status()).toBe(201);
  const createdBody = await created.text();
  const createdEtag = created.headers()["etag"];
  const memberUrl = onServer(created.headers()["location"]);
  const createdId = memberUrl.match(/\/posts\/(\d+)$/)?.[1];
  const createdSlug = createdBody.match(/<j:slug>([^<]+)<\/j:slug>/)?.[1];
  expect(createdEtag).toBeTruthy();
  expect(createdId).toBeTruthy();
  expect(createdSlug).toBeTruthy();
  expect(memberUrl).toContain(`/atompub/${username}/posts/`);

  // 3. Fetch the native Org source. Known metadata is canonicalized away, but
  // unrecognized Org directives remain source content.
  const member = await request.get(memberUrl, {
    headers: { authorization: auth },
  });
  expect(member.status()).toBe(200);
  const memberBody = await member.text();
  expect(memberBody).toContain("<title>Atom title</title>");
  expect(memberBody).toContain('term="header-category"');
  expect(memberBody).toContain("<summary>Header summary</summary>");
  expect(memberBody).toContain('type="text/org"');
  expect(memberBody).toContain("#+UNKNOWN: preserved");
  expect(memberBody).toContain("Org body");
  expect(memberBody).not.toContain("#+TITLE: Header title");
  expect(memberBody).not.toContain("#+KEYWORDS:");
  expect(memberBody).not.toContain("#+DESCRIPTION:");
  expect(memberBody).not.toContain("JAUNDER_STATUS");

  // 4. List the collection feed.
  const list = await request.get(`${BASE_URL}/atompub/${username}/posts`, {
    headers: { authorization: auth },
  });
  expect(list.status()).toBe(200);
  const listBody = await list.text();
  expect(listBody).toContain("<feed");
  expect(listBody).toContain('rel="edit"');

  // 5. Update with matching bookkeeping and a separate matching If-Match.
  const editedSlug = "atom-edited";
  // Explicit Atom title/category/summary still win over the Org header.
  const edited = await request.put(memberUrl, {
    headers: { ...xml, "if-match": createdEtag },
    data: `<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Atom edited</title>
  <category term="atom-category"/>
  <summary>Atom summary</summary>
  <content type="text/org">#+TITLE: Header edited
#+KEYWORDS: header-edited
#+DESCRIPTION: Header edited summary
#+PROPERTY: JAUNDER_STATUS draft
#+PROPERTY: JAUNDER_FORMAT org
#+PROPERTY: JAUNDER_SLUG ${editedSlug}
#+PROPERTY: JAUNDER_ID ${createdId}
#+PROPERTY: JAUNDER_SYNCED ${createdEtag}
#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-26T12:00:00Z
#+UNKNOWN: preserved

Edited Org body</content>
</entry>`,
  });
  expect(edited.status()).toBe(200);
  const editedBody = await edited.text();
  const editedEtag = edited.headers()["etag"];
  expect(editedEtag).toBeTruthy();
  expect(editedBody).toContain("<title>Atom edited</title>");
  expect(editedBody).toContain(`<j:slug>${editedSlug}</j:slug>`);
  expect(editedBody).toContain('term="atom-category"');
  expect(editedBody).toContain("<summary>Atom summary</summary>");
  expect(editedBody).toContain("#+UNKNOWN: preserved");
  expect(editedBody).toContain("Edited Org body");
  expect(editedBody).not.toContain("JAUNDER_SYNCED");

  // The fresh transport precondition isolates the stale in-body sync marker.
  // A rejected PUT must leave the accepted revision unchanged.
  const stale = await request.put(memberUrl, {
    headers: { ...xml, "if-match": editedEtag },
    data: `<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Should not persist</title>
  <content type="text/org">#+PROPERTY: JAUNDER_STATUS draft
#+PROPERTY: JAUNDER_FORMAT org
#+PROPERTY: JAUNDER_ID ${createdId}
#+PROPERTY: JAUNDER_SYNCED "stale"
#+PROPERTY: JAUNDER_SYNCED_AT 2026-08-26T12:00:00Z

Rejected body</content>
</entry>`,
  });
  expect(stale.status()).toBe(412);
  const unchanged = await request.get(memberUrl, {
    headers: { authorization: auth },
  });
  expect(unchanged.status()).toBe(200);
  const unchangedBody = await unchanged.text();
  expect(unchangedBody).toContain("Edited Org body");
  expect(unchangedBody).not.toContain("Rejected body");

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

  // 6b. The same upload with a Slug that needs percent-encoding. `Filename::sanitized`
  // — the Slug intake door — permits spaces, so this is a legal name, and the emitted
  // entry is where a raw space would have become a malformed `href` (and, for the
  // member URL, the entry's permanent `atom:id`). #675.
  const spaced = await request.post(`${BASE_URL}/atompub/${username}/media`, {
    headers: {
      authorization: auth,
      "content-type": "image/png",
      slug: "e2e spaced.png",
    },
    data: PNG,
  });
  expect(spaced.status()).toBe(201);
  const spacedBody = await spaced.text();
  expect(spacedBody).toContain("e2e%20spaced.png");
  // `<title>` legitimately carries the raw display name, so only URL-bearing attributes
  // are asserted — none may contain whitespace.
  expect(spacedBody).not.toMatch(/(?:href|src)="[^"]*\s[^"]*"/);

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

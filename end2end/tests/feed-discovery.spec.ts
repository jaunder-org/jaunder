import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { BASE_URL, goto } from "./helpers";
import { navigateInApp } from "./navigate";
import { randomUUID } from "node:crypto";
import { applySeededSession, seedUserViaTool } from "./seed";
import { createPostViaApi } from "./posts";

const markerName = "Syndication feeds";

type MarkerTrace = Window & { __feedMarkerHrefs?: string[] };

async function observeMarkerHrefs(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const hrefs: string[] = [];
    (window as MarkerTrace).__feedMarkerHrefs = hrefs;
    const record = () => {
      for (const link of document.querySelectorAll(
        'a[aria-label="Syndication feeds"]',
      )) {
        hrefs.push(link.getAttribute("href") ?? "");
      }
    };
    new MutationObserver(record).observe(document, {
      attributes: true,
      childList: true,
      subtree: true,
    });
  });
}

test("Local RSS marker opens the contextual Syndication Feed index", async ({
  page,
}) => {
  await goto(page, "/");
  const marker = page.getByRole("link", { name: markerName });
  await expect(marker).toHaveAttribute("href", "/feeds");
  await navigateInApp(
    page,
    async () => {
      await marker.focus();
      await page.keyboard.press("Enter");
    },
    {
      url: "/feeds",
      ready: "main h1:has-text('Syndication feeds for Local')",
    },
  );
  const index = page.locator("main");
  for (const [label, href, contentType] of [
    ["RSS", "/feed.rss", "application/rss+xml"],
    ["Atom", "/feed.atom", "application/atom+xml"],
    ["JSON Feed", "/feed.json", "application/feed+json"],
  ]) {
    await expect(
      index.getByRole("link", { name: label, exact: true }),
    ).toHaveAttribute("href", href);
    const response = await page.request.get(`${BASE_URL}${href}`);
    expect(response.status()).toBe(200);
    expect(response.headers()["content-type"]).toContain(contentType);
  }
  await expect(page.getByRole("link", { name: markerName })).toHaveCount(0);
  expect(new URL(page.url()).origin).toBe(BASE_URL);
});

test("Local feed index is directly available without Posts", async ({
  page,
}) => {
  await goto(page, "/feeds");
  await expect(
    page.locator("main h1", { hasText: "Syndication feeds for Local" }),
  ).toBeVisible();
  await expect(page.locator('main a[href="/feed.rss"]')).toHaveText("RSS");
});

for (const [timeline, destination, heading, feed] of [
  [
    "/tags/unused",
    "/tags/unused/feeds",
    "site tag #unused",
    "/tags/unused/feed",
  ],
  ["/~nobody", "/~nobody/feeds", "User ~nobody", "/~nobody/feed"],
] as const) {
  test(`empty ${timeline} timeline discovers its matching three formats`, async ({
    page,
  }) => {
    await goto(page, timeline);
    const marker = page.getByRole("link", { name: markerName });
    await expect(marker).toHaveAttribute("href", destination);
    await navigateInApp(page, () => marker.click(), {
      url: destination,
      ready: `main h1:has-text('Syndication feeds for ${heading}')`,
    });
    for (const extension of ["rss", "atom", "json"]) {
      await expect(
        page.locator(`main a[href="${feed}.${extension}"]`),
      ).toHaveCount(1);
    }
  });
}

for (const [path, context, feed] of [
  ["/tags/unused/feeds", "site tag #unused", "/tags/unused/feed.rss"],
  ["/~nobody/feeds", "User ~nobody", "/~nobody/feed.rss"],
] as const) {
  test(`${path} renders its contextual index on a direct visit`, async ({
    page,
  }) => {
    await goto(page, path);
    await expect(page.locator("main h1")).toHaveText(
      `Syndication feeds for ${context}`,
    );
    await expect(page.locator(`main a[href="${feed}"]`)).toHaveText("RSS");
  });
}

test("signed-in viewer finds User-tag feeds without changing the marker", async ({
  page,
  context,
}) => {
  const username = `discover${randomUUID().replaceAll("-", "").slice(0, 10)}`;
  const session = await seedUserViaTool(username, "discovery-test-password");
  await applySeededSession(context, session);
  const timeline = `/~${username}/tags/unused`;
  const destination = `${timeline}/feeds`;
  await observeMarkerHrefs(page);
  await goto(page, timeline);
  const marker = page.getByRole("link", { name: markerName });
  await expect(marker).toHaveAttribute("href", destination);
  await navigateInApp(page, () => marker.click(), {
    url: destination,
    ready: `main h1:has-text('User ~${username} tag #unused')`,
  });
  await expect(
    page.locator(`main a[href="/~${username}/tags/unused/feed.rss"]`),
  ).toHaveText("RSS");
  await expect(page.locator('head link[rel="EditURI"]')).toHaveCount(0);
  const seen = await page.evaluate(
    () => (window as MarkerTrace).__feedMarkerHrefs,
  );
  expect(seen?.length).toBeGreaterThan(0);
  expect(new Set(seen ?? [])).toEqual(new Set([destination]));
});

test("User-tag discovery renders on direct visit for an existing User", async ({
  page,
}) => {
  const username = `discover${randomUUID().replaceAll("-", "").slice(0, 10)}`;
  await seedUserViaTool(username, "discovery-test-password");
  await goto(page, `/~${username}/tags/unused/feeds`);
  await expect(page.locator("main h1")).toHaveText(
    `Syndication feeds for User ~${username} tag #unused`,
  );
  await expect(
    page.locator(`main a[href="/~${username}/tags/unused/feed.atom"]`),
  ).toHaveText("Atom");
});

test("unknown User-tag does not invent a discovery index", async ({ page }) => {
  await goto(page, "/~nobody/tags/unused/feeds");
  await expect(page.locator("main p.error")).toHaveText("user not found");
  await expect(page.getByRole("link", { name: markerName })).toHaveCount(0);
});

test("private Home has no contextual feed marker", async ({
  page,
  context,
}) => {
  const username = `discover${randomUUID().replaceAll("-", "").slice(0, 10)}`;
  const session = await seedUserViaTool(username, "discovery-test-password");
  await applySeededSession(context, session);
  await goto(page, "/app");
  await expect(page.getByRole("link", { name: markerName })).toHaveCount(0);
});

test("authenticated root redirects before a Local feed marker can paint", async ({
  page,
  context,
}) => {
  const username = `discover${randomUUID().replaceAll("-", "").slice(0, 10)}`;
  const session = await seedUserViaTool(username, "discovery-test-password");
  await applySeededSession(context, session);
  await observeMarkerHrefs(page);
  await goto(page, "/");
  await expect(page).toHaveURL(`${BASE_URL}/app`);
  await expect(page.getByRole("link", { name: markerName })).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as MarkerTrace).__feedMarkerHrefs),
  ).toEqual([]);
});

test("a Post permalink has no contextual feed marker", async ({
  page,
  context,
}) => {
  const username = `discover${randomUUID().replaceAll("-", "").slice(0, 10)}`;
  const session = await seedUserViaTool(username, "discovery-test-password");
  await applySeededSession(context, session);
  const post = await createPostViaApi(page, {
    body: "# Discovery marker exclusion",
  });
  await goto(page, post.permalink);
  await expect(page.getByRole("link", { name: markerName })).toHaveCount(0);
});

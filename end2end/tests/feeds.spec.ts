import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import { goto, register, click, waitForHydration, BASE_URL } from "./helpers";
import { setTestBudget, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { readPingLines, waitForPingMatching } from "./websub";
import { SEL } from "./selectors";

const FORMATS: { ext: string; mime: string }[] = [
  { ext: "rss", mime: "application/rss+xml" },
  { ext: "atom", mime: "application/atom+xml" },
  { ext: "json", mime: "application/feed+json" },
];

async function publishPost(
  page: Page,
  title: string,
): Promise<{ post_id: number; permalink: string }> {
  const res = await page.request.post(`${BASE_URL}/api/create_post`, {
    data: {
      body: `# ${title}\n\nBody for ${title}`,
      format: "markdown",
      slug_override: null,
      publish: true,
    },
  });
  expect(res.ok(), `create_post for "${title}"`).toBeTruthy();
  return (await res.json()) as { post_id: number; permalink: string };
}

// The feed cache is eventually consistent: a published post is visible
// immediately on a cache miss, but the background worker can cache an earlier
// snapshot (e.g. between two publishes), so reads may lag until the worker
// regenerates. Poll until the feed reflects `marker`, then return the body and
// content-type for assertions.
async function fetchFeedContaining(
  page: Page,
  url: string,
  marker: string,
  timeoutMs = 25_000,
): Promise<{ body: string; contentType: string }> {
  const deadline = Date.now() + timeoutMs;
  let lastBody = "";
  while (Date.now() < deadline) {
    const res = await page.request.get(url);
    if (res.status() === 200) {
      lastBody = await res.text();
      if (lastBody.includes(marker)) {
        return {
          body: lastBody,
          contentType: res.headers()["content-type"] ?? "",
        };
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(
    `feed ${url} never contained "${marker}" within ${timeoutMs}ms; last body: ${lastBody.slice(0, 300)}`,
  );
}

test("auto-discovery links are present on site home and user timeline, and resolve", async ({
  page,
}, info) => {
  setTestBudget(60_000);
  const username = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
  );

  // Test site home feed discovery
  await goto(page, "/");
  const homeLinks = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => ({
      href: (e as HTMLLinkElement).href,
      type: (e as HTMLLinkElement).type,
    })),
  );

  // Verify all three formats exist on home
  for (const fmt of FORMATS) {
    const link = homeLinks.find((l) => l.type === fmt.mime);
    expect(link, `${fmt.mime} on /`).toBeTruthy();
    const res = await page.request.get(link!.href);
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain(fmt.mime);
  }

  // Test user timeline feed discovery (canonical user URL is ~-prefixed)
  await goto(page, `/~${username}`);
  const userLinks = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => ({
      href: (e as HTMLLinkElement).href,
      type: (e as HTMLLinkElement).type,
    })),
  );

  // Verify all three formats exist on user timeline
  for (const fmt of FORMATS) {
    const link = userLinks.find((l) => l.type === fmt.mime);
    expect(link, `${fmt.mime} on /~${username}`).toBeTruthy();
    const res = await page.request.get(link!.href);
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain(fmt.mime);
  }
});

// M8.8.1: Two users each have their own per-user feed, in all three formats,
// containing their own posts in reverse-chronological order and excluding the
// other user's posts.
test("per-user feeds contain only that user's posts, newest first, in all formats", async ({
  page,
}, info) => {
  setTestBudget(150_000);

  const alice = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
  );
  // Alice publishes two posts; the second is newer (higher post_id) and must
  // appear first in her feed.
  await publishPost(page, "Alice Older");
  await publishPost(page, "Alice Newer");

  // Log Alice out before registering Bob. Without this, register()'s
  // success-wait (a[href='/logout']) resolves instantly against Alice's
  // still-present link, so Bob's session may not be active when we publish —
  // and Bob's post would be authored by Alice.
  await click(page, SEL.logoutLink);
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });
  await waitForHydration(page);

  const bob = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
  );
  await publishPost(page, "Bob Solo");

  for (const fmt of FORMATS) {
    // Poll until the worker has regenerated Alice's feed with her full post
    // set (newest post present), then assert order and cross-user isolation.
    const aliceFeed = await fetchFeedContaining(
      page,
      `${BASE_URL}/~${alice}/feed.${fmt.ext}`,
      "Alice Newer",
    );
    expect(aliceFeed.contentType, `alice ${fmt.ext} content-type`).toContain(
      fmt.mime,
    );

    const olderIdx = aliceFeed.body.indexOf("Alice Older");
    const newerIdx = aliceFeed.body.indexOf("Alice Newer");
    expect(olderIdx, `alice ${fmt.ext} has older post`).toBeGreaterThan(-1);
    expect(newerIdx, `alice ${fmt.ext} newest-first`).toBeLessThan(olderIdx);
    expect(aliceFeed.body, `alice ${fmt.ext} excludes bob`).not.toContain(
      "Bob Solo",
    );

    const bobFeed = await fetchFeedContaining(
      page,
      `${BASE_URL}/~${bob}/feed.${fmt.ext}`,
      "Bob Solo",
    );
    expect(bobFeed.body, `bob ${fmt.ext} excludes alice`).not.toContain(
      "Alice Newer",
    );
  }
});

// M8.8.2: With a WebSub hub configured (seeded into site_config), publishing a
// post produces a hub ping for the author's feed, and a subsequent edit
// produces a second ping. Pings are observed via the file-capture client.
test("publishing and editing a post each trigger a WebSub hub ping", async ({
  page,
}, info) => {
  setTestBudget(90_000);

  const username = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
  );
  const isUserFeed = (feedUrl: string) =>
    feedUrl.includes(`/~${username}/feed`);

  const beforePublish = readPingLines().length;
  const { post_id } = await publishPost(page, "Ping On Publish");
  const firstPing = await waitForPingMatching(
    beforePublish,
    isUserFeed,
    40_000,
  );
  expect(firstPing.feed_url).toContain(`/~${username}/feed`);

  // Let the first ping wave fully settle before snapshotting for the edit, so
  // leftover publish-wave pings are not mistaken for the edit's ping.
  await page.waitForTimeout(2_000);
  const beforeEdit = readPingLines().length;

  const editRes = await page.request.post(`${BASE_URL}/api/update_post`, {
    data: {
      post_id,
      body: "# Ping On Publish\n\nEdited body",
      format: "markdown",
      slug_override: null,
      publish: true,
    },
  });
  expect(editRes.ok(), "update_post").toBeTruthy();

  const secondPing = await waitForPingMatching(beforeEdit, isUserFeed, 40_000);
  expect(secondPing.feed_url).toContain(`/~${username}/feed`);
});

// M8.8.3: Conditional GET short-circuit — a feed fetch returns an ETag, and a
// refetch with If-None-Match returns 304 with an empty body.
test("feed honors If-None-Match with a 304 and empty body", async ({
  page,
}, info) => {
  setTestBudget(60_000);

  const username = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
  );
  await publishPost(page, "Conditional Get Post");

  const feedUrl = `${BASE_URL}/~${username}/feed.rss`;
  const first = await page.request.get(feedUrl);
  expect(first.status()).toBe(200);
  const etag = first.headers()["etag"];
  expect(etag, "feed response has ETag").toBeTruthy();

  const second = await page.request.get(feedUrl, {
    headers: { "If-None-Match": etag },
  });
  expect(second.status()).toBe(304);
  expect((await second.body()).length).toBe(0);
});

// M8.8.4: A user with no published posts still serves a valid empty feed in
// each format with a 200.
test("user with no posts serves a valid empty feed in each format", async ({
  page,
}, info) => {
  setTestBudget(60_000);

  const username = await register(
    page,
    slowBrowserFirstNavigationTimeoutMs(info, 30_000),
  );

  const rootMarkers: Record<string, string> = {
    rss: "<rss",
    atom: "<feed",
    json: "https://jsonfeed.org/version",
  };

  for (const fmt of FORMATS) {
    const res = await page.request.get(
      `${BASE_URL}/~${username}/feed.${fmt.ext}`,
    );
    expect(res.status(), `${fmt.ext} status`).toBe(200);
    expect(res.headers()["content-type"]).toContain(fmt.mime);
    const body = await res.text();
    expect(body, `${fmt.ext} is a valid feed envelope`).toContain(
      rootMarkers[fmt.ext],
    );
  }
});

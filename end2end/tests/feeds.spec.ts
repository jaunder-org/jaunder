import {
  goto,
  signInAsNewUser,
  click,
  waitForMount,
  BASE_URL,
} from "./helpers";
// `test` comes from the shared fixtures, not @playwright/test, so this spec emits
// an `e2e.test` span and its traffic — including the direct `page.request.post`
// to /api/posts/update below — is attributable to a named test (#681).
import { test, expect, setTestBudget } from "./fixtures";
import { readPingLines, waitForPingMatching } from "./websub";
// `FEED_POLL_TIMEOUT_MS` is imported, not restated: this spec derives its
// whole-test budget from it (#270), and `feeds.ts` owns the poll that consumes
// it. Two copies would let the budget silently drift from the deadline again.
import { fetchFeedContaining, FEED_POLL_TIMEOUT_MS } from "./feeds";
import { createPostViaApi } from "./posts";
import { withTimedAction } from "./actions";

const FORMATS: { ext: string; mime: string }[] = [
  { ext: "rss", mime: "application/rss+xml" },
  { ext: "atom", mime: "application/atom+xml" },
  { ext: "json", mime: "application/feed+json" },
];

/** Room for what the per-user-feeds test does *besides* polling: two seeded
 *  sessions, three `createPostViaApi` writes, and the page loads around them.
 *  Needed as an explicit term because at `workers=1` the whole-test
 *  scale is 1.0, so the budget gets no inflation from the scaler. */
const FEED_SETUP_ALLOWANCE_MS = 30_000;

/** How long the WebSub ping test waits for each hub ping to land. */
const PING_WAIT_MS = 40_000;

/** Settle window between the publish wave and the edit wave. Always fully
 *  consumed, so it counts toward that test's worst path. */
const PING_SETTLE_MS = 2_000;

/** Room for registration and the two API writes in the ping test. Deliberately
 *  thinner than `FEED_SETUP_ALLOWANCE_MS`: comfortable once the `workers>=2`
 *  scaler applies (135s total against an 82s worst path), tight at `workers=1`. */
const PING_SETUP_ALLOWANCE_MS = 8_000;

test("auto-discovery links are present on site home and user timeline, and resolve", async ({
  page,
}) => {
  const username = await signInAsNewUser(page);

  // Test site home feed discovery
  await goto(page, "/");
  const homeLinks = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => ({
      href: (e as HTMLLinkElement).href,
      type: (e as HTMLLinkElement).type,
    })),
  );

  // #198: exactly one set post-boot (three feed links, not the pre-dedupe six), and the
  // two projector stylesheet <link>s survive the marker-scoped removal (AC4).
  expect(homeLinks.length).toBe(3);
  const homeStyles = await page.$$eval(
    'head link[rel="stylesheet"]',
    (els) => els.length,
  );
  expect(homeStyles).toBe(2);

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

  // #198: one set on the user timeline too, plus exactly one RSD EditURI link.
  expect(userLinks.length).toBe(3);
  const rsd = await page.$$eval(
    'head link[rel="EditURI"]',
    (els) => els.length,
  );
  expect(rsd).toBe(1);

  // Verify all three formats exist on user timeline
  for (const fmt of FORMATS) {
    const link = userLinks.find((l) => l.type === fmt.mime);
    expect(link, `${fmt.mime} on /~${username}`).toBeTruthy();
    const res = await page.request.get(link!.href);
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain(fmt.mime);
  }
});

test("head discovery links update across a client-side nav, staying a single set", async ({
  page,
}) => {
  await signInAsNewUser(page);
  // Seed a public post carrying a tag so its footer renders a clickable tag chip.
  await createPostViaApi(page, {
    body: "# Tagged\n\nbody",
    tags: ["disco198"],
  });

  await goto(page, "/");
  await waitForMount(page);
  const siteHrefs = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => (e as HTMLLinkElement).href),
  );
  expect(siteHrefs.length).toBe(3); // one set on the Site feed

  // Client-side nav: click the post's tag chip → /tags/disco198 (leptos_router
  // intercepts the same-origin <a>, no full document load).
  await click(page, 'a.j-tag[href="/tags/disco198"]');
  await page.waitForURL(`${BASE_URL}/tags/disco198`);

  // The reactive head rewrite (old FeedDiscovery unmounts, SiteTag one mounts) lands in
  // the batch after the route change — poll until it settles rather than read once and
  // race: exactly three alternate links, all now the SiteTag feed.
  await expect
    .poll(async () =>
      page.$$eval(
        'head link[rel="alternate"]',
        (els) =>
          (els as HTMLLinkElement[]).filter((e) => e.href.includes("disco198"))
            .length,
      ),
    )
    .toBe(3);
  const tagHrefs = await page.$$eval('head link[rel="alternate"]', (els) =>
    els.map((e) => (e as HTMLLinkElement).href),
  );
  expect(tagHrefs.length).toBe(3); // exactly one set (no leftover Site links)
  expect(tagHrefs).not.toEqual(siteHrefs); // the SiteTag feed, not the Site feed
});

test("crawler path keeps the projector discovery links (no wasm)", async ({
  page,
}) => {
  // Public content so `/` renders the projector site-timeline head (an empty site falls
  // back to the link-less SPA shell). signInAsNewUser establishes the session.
  await signInAsNewUser(page);
  await createPostViaApi(page, { body: "# Crawlable\n\nbody" });
  // A raw HTTP fetch never boots wasm — the projector head is served intact.
  const res = await page.request.get(`${BASE_URL}/`);
  const html = await res.text();
  expect(html).toContain("data-jaunder-discovery");
  expect((html.match(/rel="alternate"/g) ?? []).length).toBe(3);
});

// M8.8.1: Two users each have their own per-user feed, in all three formats,
// containing their own posts in reverse-chronological order and excluding the
// other user's posts.
test("per-user feeds contain only that user's posts, newest first, in all formats", async ({
  page,
}) => {
  // Worst path: one `fetchFeedContaining` per format for each of two users, each
  // polling up to FEED_POLL_TIMEOUT_MS. Derived rather than restated so it cannot
  // drift from the deadlines it exists to cover — if the whole-test budget were
  // ever the smaller of the two, it would preempt the poll and replace its
  // diagnostic ("never contained X within Nms") with a bare timeout.
  setTestBudget(
    FORMATS.length * 2 * FEED_POLL_TIMEOUT_MS + FEED_SETUP_ALLOWANCE_MS,
  );

  const alice = await signInAsNewUser(page);
  // Alice publishes two posts; the second is newer (higher post_id) and must
  // appear first in her feed.
  await createPostViaApi(page, {
    body: "# Alice Older\n\nBody for Alice Older",
  });
  await createPostViaApi(page, {
    body: "# Alice Newer\n\nBody for Alice Newer",
  });

  // Bob's seed replaces Alice's cookie + companion cookie in place; the
  // tombstoned init script swaps the marker (spec D9) — no logout dance, and
  // the following createPostViaApi (page.request shares the context cookie
  // jar) is authored by Bob.
  const bob = await signInAsNewUser(page);
  await createPostViaApi(page, { body: "# Bob Solo\n\nBody for Bob Solo" });

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
}) => {
  // Worst path: two ping waits plus the settle = 82s. The allowance covers
  // registration and the two API writes.
  setTestBudget(2 * PING_WAIT_MS + PING_SETTLE_MS + PING_SETUP_ALLOWANCE_MS);

  const username = await signInAsNewUser(page);
  const isUserFeed = (feedUrl: string) =>
    feedUrl.includes(`/~${username}/feed`);

  const beforePublish = readPingLines().length;
  const { post_id } = await createPostViaApi(page, {
    body: "# Ping On Publish\n\nBody for Ping On Publish",
  });
  const firstPing = await waitForPingMatching(
    beforePublish,
    isUserFeed,
    PING_WAIT_MS,
  );
  expect(firstPing.feed_url).toContain(`/~${username}/feed`);

  // Let the first ping wave fully settle before snapshotting for the edit, so
  // leftover publish-wave pings are not mistaken for the edit's ping.
  //
  // The suite's only `waitForTimeout`. #794 wraps it so the wait is at least
  // *visible* in the trace (it was ~2 s of the ~18 s this test loses to
  // uninstrumented waiting); #793 replaces the sleep with a condition. The
  // duration stays `PING_SETTLE_MS` (#270), which this test's budget derives
  // from — wrapping changes attribution, not timing.
  await withTimedAction(page, "wait.settle", () =>
    page.waitForTimeout(PING_SETTLE_MS),
  );
  const beforeEdit = readPingLines().length;

  const editRes = await page.request.post(`${BASE_URL}/api/posts/update`, {
    data: {
      post_id,
      post: {
        body: "# Ping On Publish\n\nEdited body",
        format: "markdown",
        slug_override: null,
        publish: true,
      },
    },
  });
  expect(editRes.ok(), "posts::update").toBeTruthy();

  const secondPing = await waitForPingMatching(
    beforeEdit,
    isUserFeed,
    PING_WAIT_MS,
  );
  expect(secondPing.feed_url).toContain(`/~${username}/feed`);
});

// M8.8.3: Conditional GET short-circuit — a feed fetch returns an ETag, and a
// refetch with If-None-Match returns 304 with an empty body.
test("feed honors If-None-Match with a 304 and empty body", async ({
  page,
}) => {
  const username = await signInAsNewUser(page);
  await createPostViaApi(page, {
    body: "# Conditional Get Post\n\nBody for Conditional Get Post",
  });

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
}) => {
  const username = await signInAsNewUser(page);

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

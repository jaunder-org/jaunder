import {
  test,
  expect,
  slowBrowserFirstNavigationTimeoutMs,
  slowBrowserTimeoutMs,
} from "./fixtures";
import type { Browser, Page } from "@playwright/test";
import {
  BASE_URL,
  goto,
  click,
  waitForSelector,
  login,
  register,
} from "./helpers";
import { SEL } from "./selectors";

// Content Visibility — Layer A end-to-end (Task 22).
//
// Drives the whole chain through the real UI: the post-editor audience picker
// (`#audience-base` select + named-audience checkboxes), Subscribe/Unsubscribe
// on a profile, named-audience management on `/audiences`, viewer-aware read
// filtering on timelines and permalinks, and the Public-only published feed.
//
// A non-visible post resolves to the "Post not found" error page (the storage
// layer returns the same masked not-found for both private and audience-gated
// posts), so "cannot see" is asserted as `.error` "Post not found" on the
// permalink and absence of the title on the timeline.

/**
 * Register a fresh user, set a password we control, and return both so the
 * same account can be re-used across browser contexts via `login`.
 *
 * `register` generates a unique username and uses the fixed password
 * "testpassword123" (see helpers.ts), so we surface that here for re-login.
 */
async function registerKnown(
  page: Page,
  firstNavigationTimeoutMs: number,
): Promise<{ username: string; password: string }> {
  const username = await register(page, firstNavigationTimeoutMs);
  return { username, password: "testpassword123" };
}

/** Open the editor, write a titled post, pick a base audience, and publish. */
async function publishWithBaseAudience(
  page: Page,
  title: string,
  base: "public" | "subscribers" | "private",
): Promise<string> {
  await goto(page, "/posts/new");
  await waitForSelector(page, "#audience-base");
  await page.fill(SEL.postBody, `# ${title}\n\nBody for ${title}`);
  await page.selectOption("#audience-base", base);
  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);
  await expect(page.locator(SEL.saveSummary)).toContainText("Post published.");
  const href = await page
    .locator('[data-test="permalink-link"]')
    .getAttribute("href");
  expect(href, `permalink for "${title}"`).toBeTruthy();
  return href!;
}

/** Visit a permalink and assert the post body IS rendered (viewer can see it). */
async function expectPostVisible(
  browser: Browser,
  permalink: string,
  title: string,
  firstNavigationTimeoutMs: number,
  loginAs?: { username: string; password: string },
): Promise<void> {
  const ctx = await browser.newContext();
  try {
    const page = await ctx.newPage();
    if (loginAs) {
      await login(
        page,
        loginAs.username,
        loginAs.password,
        firstNavigationTimeoutMs,
      );
    }
    await goto(page, permalink, { timeout: firstNavigationTimeoutMs });
    await expect(page.locator("article h1")).toHaveText(title);
  } finally {
    await ctx.close();
  }
}

/** Visit a permalink and assert the viewer is denied (Post not found). */
async function expectPostHidden(
  browser: Browser,
  permalink: string,
  title: string,
  firstNavigationTimeoutMs: number,
  loginAs?: { username: string; password: string },
): Promise<void> {
  const ctx = await browser.newContext();
  try {
    const page = await ctx.newPage();
    if (loginAs) {
      await login(
        page,
        loginAs.username,
        loginAs.password,
        firstNavigationTimeoutMs,
      );
    }
    await goto(page, permalink, { timeout: firstNavigationTimeoutMs });
    await expect(page.locator(SEL.error)).toContainText("Post not found");
    await expect(page.locator("body")).not.toContainText(title);
  } finally {
    await ctx.close();
  }
}

// ── Scenario 1: Private ──────────────────────────────────────────────────────

test("Private post: hidden from anonymous and non-subscriber, visible to author", async ({
  page,
  browser,
}, testInfo) => {
  test.setTimeout(slowBrowserTimeoutMs(testInfo, 60_000));
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);

  const author = await registerKnown(page, firstNav);
  const permalink = await publishWithBaseAudience(
    page,
    "Private Secret",
    "private",
  );

  // A second registered user who never subscribes.
  const otherCtx = await browser.newContext();
  const otherPage = await otherCtx.newPage();
  const other = await registerKnown(otherPage, firstNav);
  await otherCtx.close();

  // Anonymous visitor: hidden on permalink.
  await expectPostHidden(browser, permalink, "Private Secret", firstNav);

  // Logged-in non-subscriber: hidden on permalink.
  await expectPostHidden(browser, permalink, "Private Secret", firstNav, other);

  // The author themselves can see it.
  await goto(page, permalink);
  await expect(page.locator("article h1")).toHaveText("Private Secret");

  // And it is absent from the author's public timeline for an anonymous viewer.
  const anonCtx = await browser.newContext();
  try {
    const anonPage = await anonCtx.newPage();
    await goto(anonPage, `/~${author.username}`, { timeout: firstNav });
    await expect(anonPage.locator("body")).not.toContainText("Private Secret");
  } finally {
    await anonCtx.close();
  }
});

// ── Scenario 2: Subscribers ──────────────────────────────────────────────────

test("Subscribers post: visible after Subscribe, hidden again after Unsubscribe", async ({
  page,
  browser,
}, testInfo) => {
  test.setTimeout(slowBrowserTimeoutMs(testInfo, 60_000));
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);

  const author = await registerKnown(page, firstNav);
  const permalink = await publishWithBaseAudience(
    page,
    "Subscribers Only",
    "subscribers",
  );

  // A second user, in their own context for the whole scenario.
  const viewerCtx = await browser.newContext();
  const viewerPage = await viewerCtx.newPage();
  try {
    await registerKnown(viewerPage, firstNav);

    // Before subscribing: cannot see the post.
    await goto(viewerPage, permalink, { timeout: firstNav });
    await expect(viewerPage.locator(SEL.error)).toContainText("Post not found");

    // Subscribe via the author's profile page.
    await goto(viewerPage, `/~${author.username}`);
    await click(viewerPage, 'button:has-text("Subscribe")');
    await waitForSelector(viewerPage, 'button:has-text("Unsubscribe")');

    // Now the subscriber can see it.
    await goto(viewerPage, permalink);
    await expect(viewerPage.locator("article h1")).toHaveText(
      "Subscribers Only",
    );

    // Unsubscribe via the profile page.
    await goto(viewerPage, `/~${author.username}`);
    await click(viewerPage, 'button:has-text("Unsubscribe")');
    await waitForSelector(viewerPage, 'button:has-text("Subscribe")');

    // After unsubscribing the post is hidden again.
    await goto(viewerPage, permalink);
    await expect(viewerPage.locator(SEL.error)).toContainText("Post not found");
  } finally {
    await viewerCtx.close();
  }
});

// ── Scenario 3: Named audience ───────────────────────────────────────────────
//
// Targeting model (verified from `audience_selection_to_targets` +
// `resolution_where`): a post's audience is the UNION of its targets — a viewer
// is admitted if they match ANY target. The editor picker expresses a named
// audience as `base ∪ Named(id)`; a `private` base drops the named set, so the
// least-broad base that still carries a named audience is `subscribers`. We
// therefore target the Friends post as `[Subscribers, Named(Friends)]` and keep
// the excluded user Y OUTSIDE the subscriber set, so the audience gate is the
// sole reason Y is denied. X is an active subscriber added to Friends and is
// admitted; Y (not subscribed, not in Friends) is denied.

test("Named audience: assigned member sees a Friends post; an unassigned non-member does not", async ({
  page,
  browser,
}, testInfo) => {
  test.setTimeout(slowBrowserTimeoutMs(testInfo, 90_000));
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);

  const author = await registerKnown(page, firstNav);

  // X subscribes (so the author can add X to a named audience).
  const xCtx = await browser.newContext();
  const xPage = await xCtx.newPage();
  const userX = await registerKnown(xPage, firstNav);
  await goto(xPage, `/~${author.username}`);
  await click(xPage, 'button:has-text("Subscribe")');
  await waitForSelector(xPage, 'button:has-text("Unsubscribe")');

  // Y registers but never subscribes — it is neither a subscriber nor a member.
  const yCtx = await browser.newContext();
  const yPage = await yCtx.newPage();
  const userY = await registerKnown(yPage, firstNav);

  // Author creates a "Friends" audience and adds only X.
  await goto(page, "/audiences");
  await page.fill('input[name="name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  const friends = page.locator(".j-audience-item", { hasText: "Friends" });
  await expect(friends).toBeVisible();

  // X appears in the roster as a not-yet-member (with an "Add" button). Add X.
  const xRow = friends
    .locator(".j-audience-members li")
    .filter({ hasText: userX.username });
  await expect(xRow).toBeVisible();
  await xRow.locator('button:has-text("Add")').click();
  // Once added, X's row shows a "Remove" button (the is-member state).
  await waitForSelector(
    page,
    `.j-audience-members li:has-text("${userX.username}") button:has-text("Remove")`,
  );

  // Author publishes a post targeted to subscribers + the Friends audience.
  await goto(page, "/posts/new");
  await waitForSelector(page, "#audience-base");
  await page.fill(SEL.postBody, "# Friends Post\n\nBody for Friends Post");
  await page.selectOption("#audience-base", "subscribers");
  await page
    .locator("label", { hasText: "Friends" })
    .locator('input[type="checkbox"]')
    .check();
  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);
  await expect(page.locator(SEL.saveSummary)).toContainText("Post published.");
  const friendsPermalink = await page
    .locator('[data-test="permalink-link"]')
    .getAttribute("href");
  expect(friendsPermalink).toBeTruthy();

  // X (subscriber + Friends member) can see it.
  await goto(xPage, friendsPermalink!);
  await expect(xPage.locator("article h1")).toHaveText("Friends Post");

  // Y (not subscribed, not in Friends) cannot.
  await goto(yPage, friendsPermalink!);
  await expect(yPage.locator(SEL.error)).toContainText("Post not found");
  await expect(yPage.locator("body")).not.toContainText("Friends Post");

  expect(userY.username).not.toEqual(userX.username);

  await xCtx.close();
  await yCtx.close();
});

// ── Scenario 4: Public + published feed ──────────────────────────────────────

test("Public post is visible to anonymous and appears in the feed; Subscribers post does not", async ({
  page,
  browser,
}, testInfo) => {
  test.setTimeout(slowBrowserTimeoutMs(testInfo, 90_000));
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);

  const author = await registerKnown(page, firstNav);

  const publicPermalink = await publishWithBaseAudience(
    page,
    "Public Broadcast",
    "public",
  );
  await publishWithBaseAudience(page, "Feed Subscribers Only", "subscribers");

  // Anonymous visitor sees the public post on its permalink.
  await expectPostVisible(
    browser,
    publicPermalink,
    "Public Broadcast",
    firstNav,
  );

  // The published feed contains the Public post and excludes the Subscribers one.
  // The feed is eventually consistent, so poll until the public marker appears.
  const feedUrl = `${BASE_URL}/~${author.username}/feed.atom`;
  const deadline = Date.now() + 25_000;
  let body = "";
  while (Date.now() < deadline) {
    const res = await page.request.get(feedUrl);
    if (res.status() === 200) {
      body = await res.text();
      if (body.includes("Public Broadcast")) {
        break;
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  expect(body, "feed contains the Public post").toContain("Public Broadcast");
  expect(body, "feed excludes the Subscribers-only post").not.toContain(
    "Feed Subscribers Only",
  );
});

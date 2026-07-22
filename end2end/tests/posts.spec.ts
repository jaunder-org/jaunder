import {
  test,
  expect,
  setTestBudget,
  slowBrowserFirstNavigationTimeoutMs,
  slowBrowserTimeoutMs,
} from "./fixtures";
import { goto, click, waitForSelector, register } from "./helpers";
import { createPerfProbe } from "./perf";
import { seedPostsViaTool } from "./seed";
import { SEL } from "./selectors";
import { composePost, createPostViaApi } from "./posts";

const TIMELINE_PAGE_SIZE = 50;
const TIMELINE_OVERFLOW_COUNT = 1;
const LOCAL_TIMELINE_AUTHOR_COUNT = 26;
const HOME_FEED_SELF_COUNT = 51;
const HOME_FEED_OTHER_COUNT = 2;

test("authenticated user can create a post through the UI", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  await expect(page.locator(SEL.topbarHeading)).toHaveText("New post");
  await page.fill(SEL.postBody, "# Playwright Post\n\n**browser**");
  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);

  await expect(page.locator(SEL.saveSummary)).toContainText("Post published.");
  await expect(page.locator(SEL.saveSummary)).toContainText(
    "Slug: playwright-post",
  );
});

test("authenticated user can create a post with a summary", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  await expect(page.locator(SEL.topbarHeading)).toHaveText("New post");
  await page.fill(SEL.postBody, "# Summary Test\n\nBody text");
  await page.fill("#compose-summary", "This is a summary");
  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);

  await expect(page.locator(SEL.saveSummary)).toContainText("Post published.");

  const permalinkLink = page.locator('[data-test="permalink-link"]');
  const permalinkHref = await permalinkLink.getAttribute("href");
  expect(permalinkHref).toBeTruthy();

  await goto(page, permalinkHref!);

  await expect(page.locator("article h1")).toHaveText("Summary Test");
  await expect(page.locator("article")).toContainText("This is a summary");
});

// #545: an over-long summary (> MAX_POST_SUMMARY_CHARS = 500) is rejected
// client-side by the shared PostSummary FromStr — the newtype's own message shows
// inline once touched, and the publish button is disabled (ADR-0065
// disable-until-valid, gated on summary validity alongside the slug).
test("over-long post summary shows an inline error and gates submit", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  await page.fill(SEL.postBody, "# Over Cap\n\nBody text");
  const summaryInput = page.locator("#compose-summary");
  await summaryInput.fill("a".repeat(501));
  await summaryInput.blur();

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.publishButton("true"))).toBeDisabled();
});

// #545: a summary set on create can be cleared on edit. Under the typed
// Option<PostSummary> wire arg an emptied summary is omitted (dispatched as None),
// persisting as cleared — verified in the browser: create with a summary, edit,
// empty the field, save, then reopen the editor and confirm it is empty.
test("clearing a post summary on edit persists as empty", async ({
  registeredPage: page,
}) => {
  test.slow();
  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, "# Clearable\n\nbody");
  await page.fill("#compose-summary", "A summary to remove");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  const summary = page.locator(SEL.saveSummary);
  // Preview is gone (#24): reach the post at its canonical permalink, then read
  // the post_id off the PostCard's Edit affordance.
  const permalinkHref = (await summary
    .locator('[data-test="permalink-link"]')
    .getAttribute("href"))!;
  expect(permalinkHref).toBeTruthy();
  await goto(page, permalinkHref);
  const editLink = page.locator('.j-post-acts a:has-text("Edit")');
  await editLink.waitFor();
  const postId = (await editLink.getAttribute("href"))!.match(
    /\/posts\/(\d+)\/edit/,
  )![1];

  // Edit: the summary prefills; clear it and save.
  await goto(page, `/posts/${postId}/edit`);
  await expect(page.locator("#edit-summary")).toHaveValue(
    "A summary to remove",
  );
  await page.fill("#edit-summary", "");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  // Reopen the editor: the summary is now empty (cleared via None-omission).
  await goto(page, `/posts/${postId}/edit`);
  await expect(page.locator("#edit-summary")).toHaveValue("");
});

test("authenticated user can save a draft through the UI", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  await page.fill(SEL.postBody, "*draft*");
  await click(page, '.j-seg button:has-text("Org")');
  await page.fill('input[name="slug_override"]', "Draft-Slug");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  await expect(page.locator(SEL.saveSummary)).toContainText("Draft saved.");
  await expect(page.locator(SEL.saveSummary)).toContainText("Slug: draft-slug");
});

test("published post renders at permalink", async ({
  registeredPage: page,
}) => {
  test.slow();
  const summary = await composePost(page, {
    body: "# Permalink Story\n\n**hello permalink**",
    publish: true,
  });
  await expect(summary).toContainText("Post published.");

  const slugAttr = await summary
    .locator('[data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(slugAttr).toBeTruthy();

  const permalinkLink = summary.locator('[data-test="permalink-link"]');
  await expect(permalinkLink).toBeVisible();
  const permalinkHref = await permalinkLink.getAttribute("href");
  expect(permalinkHref).toBeTruthy();

  const targetUrl = permalinkHref!;

  await goto(page, targetUrl);

  await expect(page.locator("article h1")).toHaveText("Permalink Story");
  await expect(page.locator(".j-post-body")).toContainText("hello permalink");
});

test("authenticated user can edit a draft post", async ({
  registeredPage: page,
}) => {
  test.slow();
  // Create a draft; title embedded as # heading
  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, "# Original Draft\n\noriginal body");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  const summary = page.locator(SEL.saveSummary);
  // Preview is gone (#24): reach the draft at its canonical permalink, then read
  // the post_id off the PostCard's Edit affordance.
  const permalinkHref = (await summary
    .locator('[data-test="permalink-link"]')
    .getAttribute("href"))!;
  expect(permalinkHref).toBeTruthy();
  await goto(page, permalinkHref);
  const editLink = page.locator('.j-post-acts a:has-text("Edit")');
  await editLink.waitFor();
  const postId = (await editLink.getAttribute("href"))!.match(
    /\/posts\/(\d+)\/edit/,
  )![1];

  // Navigate to edit page
  await goto(page, `/posts/${postId}/edit`);

  await expect(page.locator(SEL.topbarHeading)).toHaveText("Edit Post");

  // Update the draft; keep heading to preserve the slug
  await page.fill(SEL.postBody, "# Original Draft\n\n**edited content**");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  await expect(page.locator(SEL.saveSummary)).toContainText("Draft saved.");
  await expect(page.locator(SEL.saveSummary)).toContainText(
    "Draft saved.Slug: original-draftView post",
  );
});

test("editing an invalid or nonexistent post shows not-found", async ({
  registeredPage: page,
}) => {
  // Unparseable post_id ("abc") drives the #487 `None` arm: `post_id_param`
  // yields None and the fetcher short-circuits to a client-side "Post not found"
  // with no server lookup. This is the path the sentinel removal introduced, so
  // it must be guarded against regressing to a sentinel id / wasted round-trip.
  await goto(page, "/posts/abc/edit");
  await expect(page.locator(SEL.error)).toContainText("Post not found");

  // A well-formed but nonexistent id takes the unchanged `Some(id)` server path
  // and must still surface the same message (parity with pre-#487 behavior).
  await goto(page, "/posts/999999999/edit");
  await expect(page.locator(SEL.error)).toContainText("Post not found");
});

test("editing a published post freezes the slug", async ({
  registeredPage: page,
}) => {
  test.slow();
  // Create and publish a post; title embedded as # heading
  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, "# Published Article\n\noriginal content");
  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);

  const summary = page.locator(SEL.saveSummary);
  const originalSlug = await summary
    .locator('[data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(originalSlug).toBeTruthy();

  // Preview is gone (#24): reach the published post at its permalink, then read
  // the post_id off the PostCard's Edit affordance.
  const permalinkHref = (await summary
    .locator('[data-test="permalink-link"]')
    .getAttribute("href"))!;
  expect(permalinkHref).toBeTruthy();
  await goto(page, permalinkHref);
  const editLink = page.locator('.j-post-acts a:has-text("Edit")');
  await editLink.waitFor();
  const postId = (await editLink.getAttribute("href"))!.match(
    /\/posts\/(\d+)\/edit/,
  )![1];

  // Navigate to edit page
  await goto(page, `/posts/${postId}/edit`);

  // Published post should not have a slug_override input
  await expect(page.locator('input[name="slug_override"]')).not.toBeVisible();

  // Save the published post (body already pre-filled from loaded post; slug stays frozen)
  await click(page, SEL.publishButton("true"));
  // After save, editor redirects to the permalink page
  await waitForSelector(page, "article h1");
  expect(page.url()).toContain(originalSlug!);
});

test("draft lifecycle: create, view, edit, and publish", async ({
  page,
  context,
}, testInfo) => {
  const firstNavigationTimeoutMs = slowBrowserFirstNavigationTimeoutMs(
    testInfo,
    12_000,
  );
  await register(page, firstNavigationTimeoutMs);

  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, "# Lifecycle Draft\n\ninitial draft body");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  // Preview is gone (#24): the drafts listing links only the canonical permalink
  // and carries the Edit affordance — derive both from the row (AC5).
  await goto(page, "/drafts");
  const initialDraftRow = page.locator("li", { hasText: "Lifecycle Draft" });
  await expect(initialDraftRow).toBeVisible();
  await expect(initialDraftRow.locator('a:has-text("Preview")')).toHaveCount(0);
  const permalinkHref = await initialDraftRow
    .locator('a:has-text("Permalink")')
    .getAttribute("href");
  expect(permalinkHref).toBeTruthy();
  const permalinkUrl = permalinkHref!;
  const editHref = await initialDraftRow
    .locator('a:has-text("Edit")')
    .getAttribute("href");
  expect(editHref).toBeTruthy();

  await goto(page, editHref!);
  await page.fill(SEL.postBody, "# Lifecycle Draft\n\nedited draft body");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  await goto(page, permalinkUrl);
  // The permalink is a fresh CSR mount; under worker CPU contention (#155,
  // workers=4) its render can exceed the global 5s expect timeout, so scale
  // these post-navigation assertions with the same contention factor the
  // test-level budget uses.
  const bodyRenderTimeoutMs = slowBrowserTimeoutMs(testInfo, 5_000);
  await expect(page.locator(".j-post-body")).toContainText(
    "edited draft body",
    {
      timeout: bodyRenderTimeoutMs,
    },
  );
  await expect(page.locator(".draft-banner")).toContainText(
    "Draft - visible only to you",
    { timeout: bodyRenderTimeoutMs },
  );
  // #23/#24: the draft's PostCard offers Publish, never Unpublish (AC2).
  const draftActs = page.locator(".j-post-acts");
  await expect(draftActs.locator('button:has-text("Publish")')).toBeVisible();
  await expect(draftActs.locator('button:has-text("Unpublish")')).toHaveCount(
    0,
  );

  const guestContext = await context.browser()!.newContext();
  const guestPage = await guestContext.newPage();
  await goto(guestPage, permalinkUrl, { timeout: firstNavigationTimeoutMs });
  await expect(guestPage.locator("body")).not.toContainText(
    "edited draft body",
  );
  await guestContext.close();

  // Publish from the post's own permalink via the draft-aware PostCard (#23/#24):
  // Publish sits behind a confirm and, on success, navigates to the canonical
  // published permalink — where the post renders without the draft banner (AC4).
  await goto(page, permalinkUrl);
  page.once("dialog", (dialog) => dialog.accept());
  await page.locator('.j-post-acts button:has-text("Publish")').click();
  await expect(page.locator(".j-post-body")).toContainText(
    "edited draft body",
    {
      timeout: bodyRenderTimeoutMs,
    },
  );
  await expect(page.locator(".draft-banner")).toHaveCount(0);
});

test("per-user timeline lists published posts with pagination", async ({
  page,
  firstNav,
}, testInfo) => {
  const perf = createPerfProbe(testInfo, "user_timeline_pagination");

  const username = await register(page, firstNav);

  await perf.timed("seed_posts", async () => {
    seedPostsViaTool(
      username,
      TIMELINE_PAGE_SIZE + TIMELINE_OVERFLOW_COUNT,
      "Timeline Post",
    );
  });

  await goto(page, `/~${username}`, { timeout: firstNav });

  await expect(page.locator("h1", { hasText: /^Posts by / })).toContainText(
    `Posts by ${username}`,
  );
  await expect(page.locator("article.j-post")).toHaveCount(TIMELINE_PAGE_SIZE);
  await expect(page.locator("article.j-post").first()).toContainText(
    `Timeline Post ${TIMELINE_PAGE_SIZE}`,
  );

  await click(page, 'button:has-text("Load more")');
  perf.mark("load_more_clicked");
  await expect(page.locator("article.j-post")).toHaveCount(
    TIMELINE_PAGE_SIZE + TIMELINE_OVERFLOW_COUNT,
    {
      timeout: 10_000,
    },
  );
  await expect(page.locator("article.j-post").last()).toContainText(
    "Timeline Post 0",
    { timeout: 10_000 },
  );
  perf.mark("assertions_complete");
  await perf.log({ username });
});

test("home page shows local timeline for unauthenticated users", async ({
  page,
  browser,
  firstNav,
}, testInfo) => {
  const perf = createPerfProbe(testInfo, "home_local_timeline");

  await perf.timed("seed_author_one", async () => {
    const u1 = await register(page, firstNav);
    seedPostsViaTool(u1, LOCAL_TIMELINE_AUTHOR_COUNT, "Local Author One");
  });

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await perf.timed("seed_author_two", async () => {
    const u2 = await register(secondPage, firstNav);
    seedPostsViaTool(u2, LOCAL_TIMELINE_AUTHOR_COUNT, "Local Author Two");
  });

  const guestContext = await browser.newContext();
  const guestPage = await guestContext.newPage();
  await goto(guestPage, "/", { timeout: firstNav });

  // Site title is still the seeded value: admin-site (the only mutator) runs in
  // the serial Playwright project and never overlaps this test.
  await expect(guestPage.locator(SEL.topbarHeading)).toHaveText(
    "jaunder.local",
  );

  // Own-scoped: with workers>1 other tests publish into the same global local
  // timeline, so assert a full first page exists rather than an exact count.
  // This test alone seeds 2 * LOCAL_TIMELINE_AUTHOR_COUNT (52) posts, so a full
  // page is guaranteed regardless of concurrent publishers.
  await expect
    .poll(async () => guestPage.locator("article.j-post").count(), {
      timeout: 10_000,
    })
    .toBeGreaterThanOrEqual(TIMELINE_PAGE_SIZE);
  const firstPageCount = await guestPage.locator("article.j-post").count();

  // Pagination works: "Load more" grows the rendered set.
  await click(guestPage, 'button:has-text("Load more")');
  perf.mark("load_more_clicked");
  await expect
    .poll(async () => guestPage.locator("article.j-post").count(), {
      timeout: 10_000,
    })
    .toBeGreaterThan(firstPageCount);
  perf.mark("assertions_complete");
  await perf.log();

  await guestContext.close();
  await secondContext.close();
});

test("cockpit /app shows the authenticated home feed with pagination", async ({
  page,
  browser,
  firstNav,
}, testInfo) => {
  const perf = createPerfProbe(testInfo, "home_authenticated_feed");

  await perf.timed("seed_self", async () => {
    const me = await register(page, firstNav);
    seedPostsViaTool(me, HOME_FEED_SELF_COUNT, "Home Feed Mine");
  });

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await perf.timed("seed_other", async () => {
    const other = await register(secondPage, firstNav);
    seedPostsViaTool(other, HOME_FEED_OTHER_COUNT, "Home Feed Other");
  });

  await goto(page, "/app", { timeout: firstNav });

  await expect(page.locator(SEL.topbarHeading)).toHaveText("Home");
  await expect(page.locator("article.j-post")).toHaveCount(TIMELINE_PAGE_SIZE);
  await expect(page.locator("article.j-post").first()).toContainText(
    `Home Feed Mine ${HOME_FEED_SELF_COUNT - 1}`,
  );
  await expect(page.locator("body")).not.toContainText("Home Feed Other");

  await click(page, 'button:has-text("Load more")');
  perf.mark("load_more_clicked");
  await expect(page.locator("article.j-post")).toHaveCount(
    HOME_FEED_SELF_COUNT,
    {
      timeout: 10_000,
    },
  );
  await expect(page.locator("body")).not.toContainText("Home Feed Other");
  perf.mark("assertions_complete");
  await perf.log();

  await secondContext.close();
});

test("authenticated user can delete a published post", async ({
  registeredPage: page,
}) => {
  test.slow();
  // Create a published post; title embedded as # heading (title input is removed from UI)
  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, "# Post To Delete\n\nthis will be deleted");
  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);

  const permalinkLink = page.locator('[data-test="permalink-link"]');
  const permalinkHref = await permalinkLink.getAttribute("href");
  expect(permalinkHref).toBeTruthy();
  const permalinkUrl = permalinkHref!;

  // Navigate to permalink page
  await goto(page, permalinkUrl);
  await expect(page.locator("article h1")).toHaveText("Post To Delete");

  // Delete button should be visible for the author
  await expect(page.locator('button:has-text("Delete")')).toBeVisible();

  // Accept the confirm dialog and click delete
  page.once("dialog", (dialog) => dialog.accept());
  await click(page, 'button:has-text("Delete")');
  await waitForSelector(page, ".success");
  await expect(page.locator(".success")).toContainText("Post deleted.");

  // Verify the permalink now returns a not-found error
  await goto(page, permalinkUrl);
  await expect(page.locator(SEL.error)).toContainText("Post not found");

  // Verify excluded from user timeline
  const username = permalinkUrl.match(/\/~([^/]+)\//)?.[1];
  expect(username).toBeTruthy();
  await goto(page, `/~${username}`);
  await expect(page.locator("body")).not.toContainText("Post To Delete");
});

test("inline composer: published post appears in timeline without page reload", async ({
  registeredPage: page,
}, testInfo) => {
  // The /app cockpit must already show the feed with the composer.
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  const initialCount = await page.locator("article.j-post").count();

  await page.fill('.j-composer textarea[name="body"]', "Live refresh test");
  await click(page, '.j-composer button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-composer p.success");

  // The new post should appear without a page reload.
  await expect(page.locator("article.j-post")).toHaveCount(initialCount + 1, {
    timeout: slowBrowserTimeoutMs(testInfo, 8_000),
  });
});

test("inline composer: plain body publishes titleless note", async ({
  registeredPage: page,
}) => {
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  await page.fill('.j-composer textarea[name="body"]', "Titleless inline note");
  await click(page, '.j-composer button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-composer p.success");

  const post = page.locator("article.j-post").first();
  await expect(post).toContainText("Titleless inline note");
  await expect(post.locator(".j-post-title")).toHaveCount(0);
});

test("inline composer: markdown heading becomes article title", async ({
  registeredPage: page,
}) => {
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  await page.fill(
    '.j-composer textarea[name="body"]',
    "# Inline Article\n\nArticle body",
  );
  await click(page, '.j-composer button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-composer p.success");

  const post = page.locator("article.j-post").first();
  await expect(post.locator(".j-post-title")).toContainText("Inline Article");
  await expect(post.locator(".j-post-body")).toContainText("Article body");
  // Body is stored verbatim, so the # heading renders as <h1> inside the body
  await expect(post.locator(".j-post-body h1")).toHaveCount(1);
});

test("inline composer: publish flash is a link to the post permalink", async ({
  registeredPage: page,
}) => {
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  await page.fill('.j-composer textarea[name="body"]', "Flash link test");
  await click(page, '.j-composer button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-composer p.success a");

  const link = page.locator(".j-composer p.success a");
  await expect(link).toContainText("Post published!");
  const href = await link.getAttribute("href");
  expect(href).toBeTruthy();
  expect(href).toMatch(/^\/~[^/]+\//);
});

test("inline composer: draft flash links to the draft's canonical permalink", async ({
  registeredPage: page,
}) => {
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  await page.fill('.j-composer textarea[name="body"]', "Draft flash link test");
  await click(page, '.j-composer button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-composer p.success a");

  const link = page.locator(".j-composer p.success a");
  await expect(link).toContainText("Draft saved!");
  const href = await link.getAttribute("href");
  expect(href).toBeTruthy();
  // #24: the flash links to the canonical permalink, never a /draft/…/preview URL.
  expect(href).toMatch(/^\/~/);
  expect(href).not.toContain("/preview");
});

test("inline composer: flash clears when user starts typing", async ({
  registeredPage: page,
}) => {
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  await page.fill('.j-composer textarea[name="body"]', "Flash clear test");
  await click(page, '.j-composer button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-composer p.success");

  // Typing in the textarea should dismiss the flash immediately.
  await page.type('.j-composer textarea[name="body"]', "x");
  await expect(page.locator(".j-composer p.success")).toHaveCount(0);
});

test("inline composer: format toggle switches active button", async ({
  registeredPage: page,
}) => {
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  // Markdown is active by default.
  const markdownBtn = page.locator('.j-seg button:has-text("Markdown")');
  const orgBtn = page.locator('.j-seg button:has-text("Org")');
  await expect(markdownBtn).toHaveClass(/is-selected/);
  await expect(orgBtn).not.toHaveClass(/is-selected/);

  // Click Org to switch.
  await click(page, '.j-seg button:has-text("Org")');
  await expect(orgBtn).toHaveClass(/is-selected/);
  await expect(markdownBtn).not.toHaveClass(/is-selected/);
});

test("create post with tags via UI: tags persist and appear on the post", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  await page.fill(SEL.postBody, "# Tagged Post\n\ncontent");

  // Add three tags via the TagInput: type and press Enter for each
  for (const tag of ["alpha", "beta", "gamma"]) {
    await page.fill(".j-tag-text", tag);
    await page.keyboard.press("Enter");
    await waitForSelector(page, `.j-tag-chip-label:has-text("#${tag}")`);
  }

  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);
  await expect(page.locator(SEL.saveSummary)).toContainText("Post published.");

  // Navigate to the permalink and confirm all three tags appear
  const permalink = await page
    .locator('[data-test="permalink-link"]')
    .getAttribute("href");
  expect(permalink).toBeTruthy();
  await goto(page, permalink!);

  const tagList = page.locator(".j-tag-list");
  await expect(tagList).toContainText("#alpha");
  await expect(tagList).toContainText("#beta");
  await expect(tagList).toContainText("#gamma");
});

test("tag chip on permalink navigates to site tag listing", async ({
  registeredPage: page,
}) => {
  // Create a published post with two tags via the API
  const { permalink } = await createPostViaApi(page, {
    body: "# Chip Nav Post\n\ncontent",
    tags: ["rustlang", "nix"],
  });
  expect(permalink).toBeTruthy();

  // Visit permalink; wait for tag chips to render
  await goto(page, permalink);
  await waitForSelector(page, '.j-tag[href="/tags/rustlang"]');

  // Click the "rustlang" chip — Leptos router handles this client-side
  await page.locator('.j-tag[href="/tags/rustlang"]').click();
  await waitForSelector(page, '.j-topbar:has-text("#rustlang")');

  // Post should appear in the listing
  await expect(page.locator(".j-page")).toContainText("Chip Nav Post");
});

test("editing a post updates tag chips and tag listing pages", async ({
  registeredPage: page,
}) => {
  setTestBudget(60_000);
  // Use tags unique to this test so cross-test pollution can't affect the
  // /tags/:tag listing checks below.
  const { permalink, post_id } = await createPostViaApi(page, {
    body: "# Tag Edit Post\n\ncontent",
    tags: ["xedita", "xeditb", "xeditc"],
  });

  // Open the edit page directly
  await goto(page, `/posts/${post_id}/edit`);

  // Wait for pre-populated chips from get_post_preview to appear
  await waitForSelector(page, '.j-tag-chip-label:has-text("#xeditc")');

  // Remove the "xeditc" chip
  await page
    .locator(".j-tag-chip")
    .filter({ hasText: "#xeditc" })
    .locator(".j-tag-chip-remove")
    .click();
  await expect(
    page.locator('.j-tag-chip-label:has-text("#xeditc")'),
  ).toHaveCount(0);

  // Add a new "xeditd" chip
  await page.fill(".j-tag-text", "xeditd");
  await page.keyboard.press("Enter");
  await waitForSelector(page, '.j-tag-chip-label:has-text("#xeditd")');

  // Save (post is already published, so the button reads "Save").
  // EditPostPage redirects via location.replace() to the permalink on success.
  await click(page, SEL.publishButton("true"));

  // Wait for something that only exists on the destination permalink page.
  // waitForHydration() would race in Firefox: body[data-hydrated] is already
  // set on the (hydrated) edit page, so page.evaluate() runs while Firefox is
  // mid-navigation and the execution context gets destroyed.
  await waitForSelector(page, ".j-tag-list");

  // Now on the permalink — verify the footer reflects the updated tag set
  const tagList = page.locator(".j-tag-list");
  await expect(tagList).toContainText("#xedita");
  await expect(tagList).toContainText("#xeditb");
  await expect(tagList).toContainText("#xeditd");
  await expect(tagList).not.toContainText("#xeditc");

  // /tags/xeditc should no longer list the post
  await goto(page, "/tags/xeditc");
  await waitForSelector(page, ".j-page");
  await expect(page.locator(".j-page")).toContainText(
    "No posts with this tag yet.",
  );

  // /tags/xeditd should list it
  await goto(page, "/tags/xeditd");
  await waitForSelector(page, ".j-post-body");
  await expect(page.locator(".j-post-body")).toContainText("Tag Edit Post");
});

test("TagInput autocomplete suggests existing tags", async ({
  registeredPage: page,
}) => {
  // Seed the tag corpus with a known tag
  await createPostViaApi(page, { body: "seed post", tags: ["rustlang"] });

  // Open the create form and type a prefix that matches "rustlang"
  await goto(page, "/posts/new");
  await page.fill(".j-tag-text", "rust");

  // Autocomplete dropdown should appear (150 ms debounce + fetch)
  await waitForSelector(page, ".j-tag-suggest");
  await expect(page.locator(".j-tag-suggest")).toContainText("rustlang");

  // Click the suggestion to add it as a chip
  await page.locator(".j-tag-suggest-item").first().click();
  await waitForSelector(page, '.j-tag-chip-label:has-text("#rustlang")');
});

test("TagInput: Backspace on empty input removes last chip", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");

  // Add two chips
  await page.fill(".j-tag-text", "alpha");
  await page.keyboard.press("Enter");
  await waitForSelector(page, '.j-tag-chip-label:has-text("#alpha")');
  await page.fill(".j-tag-text", "beta");
  await page.keyboard.press("Enter");
  await waitForSelector(page, '.j-tag-chip-label:has-text("#beta")');

  // Input is empty; Backspace should remove the last chip ("beta")
  await page.keyboard.press("Backspace");
  await expect(page.locator('.j-tag-chip-label:has-text("#beta")')).toHaveCount(
    0,
  );
  await expect(
    page.locator('.j-tag-chip-label:has-text("#alpha")'),
  ).toHaveCount(1);
});

test("TagInput: keyboard navigation selects autocomplete item", async ({
  registeredPage: page,
}) => {
  // Seed a known tag
  await createPostViaApi(page, { body: "seed post", tags: ["kbdnav"] });

  await goto(page, "/posts/new");
  await page.fill(".j-tag-text", "kbd");
  await waitForSelector(page, ".j-tag-suggest");

  // ArrowDown highlights first item; Enter commits it
  await page.keyboard.press("ArrowDown");
  await expect(page.locator(".j-tag-suggest-item.is-active")).toContainText(
    "kbdnav",
  );
  await page.keyboard.press("Enter");
  await waitForSelector(page, '.j-tag-chip-label:has-text("#kbdnav")');
  // Dropdown closes after selection
  await expect(page.locator(".j-tag-suggest")).toHaveCount(0);
});

test("TagInput: Escape dismisses autocomplete without adding a chip", async ({
  registeredPage: page,
}) => {
  await createPostViaApi(page, { body: "seed post", tags: ["esctest"] });

  await goto(page, "/posts/new");
  await page.fill(".j-tag-text", "esc");
  await waitForSelector(page, ".j-tag-suggest");

  await page.keyboard.press("Escape");
  await expect(page.locator(".j-tag-suggest")).toHaveCount(0);
  // No chip should have been added
  await expect(page.locator(".j-tag-chip")).toHaveCount(0);
});

test("TagInput: invalid tag text shows an error", async ({
  registeredPage: page,
}) => {
  await goto(page, "/posts/new");
  // "bad tag" has a space — rejected by TagLabel::from_str
  await page.fill(".j-tag-text", "bad tag");
  await page.keyboard.press("Enter");

  await waitForSelector(page, ".j-tag-error");
  await expect(page.locator(".j-tag-error")).toContainText(
    "tag must be non-empty",
  );
  // No chip should appear
  await expect(page.locator(".j-tag-chip")).toHaveCount(0);
});

test("authenticated user can delete a draft from the drafts page", async ({
  registeredPage: page,
}) => {
  test.slow();
  // Create a draft; title embedded as # heading (title input is removed from UI)
  await goto(page, "/posts/new");
  await page.fill(SEL.postBody, "# Draft To Delete\n\ndraft content");
  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  // Navigate to drafts page
  await goto(page, "/drafts");
  await expect(page.locator("body")).toContainText("Draft To Delete");

  // Delete the draft
  page.once("dialog", (dialog) => dialog.accept());
  await click(page, 'button:has-text("Delete")');
  await waitForSelector(page, ".success");
  await expect(page.locator(".success")).toContainText("Draft deleted.");
  await expect(page.locator("body")).not.toContainText("Draft To Delete");
});

test("scheduling a post shows a Scheduled-for badge on the drafts page", async ({
  registeredPage: page,
}) => {
  test.slow();
  // A fixed far-future wall-clock time keeps the post unambiguously *scheduled*
  // no matter when the suite runs, with no Date arithmetic that could drift.
  // The non-compact composer's optional schedule control is `#compose-publish-at`
  // (a `datetime-local` input); a future time plus Publish creates a post whose
  // `published_at` is in the future. Such posts surface on the drafts page with a
  // "Scheduled for …" badge (`.j-badge-scheduled`) rather than going live.
  const FUTURE_DATETIME_LOCAL = "2999-01-01T09:00";

  await goto(page, "/posts/new");
  await page.fill(
    SEL.postBody,
    "# Scheduled Draft\n\nbody for a scheduled post",
  );
  await page.fill("#compose-publish-at", FUTURE_DATETIME_LOCAL);
  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);

  // The scheduled post lists on /drafts (published_at in the future), marked with
  // the "Scheduled for …" badge that distinguishes it from a true draft.
  await goto(page, "/drafts");
  const scheduledRow = page.locator("li", { hasText: "Scheduled Draft" });
  await expect(scheduledRow).toBeVisible();
  const badge = scheduledRow.locator(".j-badge-scheduled");
  await expect(badge).toBeVisible();
  await expect(badge).toContainText("Scheduled for");
});

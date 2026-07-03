import {
  test,
  expect,
  hydrationHeavyFirstNavigationTimeoutMs,
  hydrationHeavyTimeoutMs,
} from "./fixtures";
import type { Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { BASE_URL, goto, click, waitForSelector, register } from "./helpers";
import { createPerfProbe } from "./perf";
import { seedPostsViaTool } from "./seed";

const TIMELINE_PAGE_SIZE = 50;
const TIMELINE_OVERFLOW_COUNT = 1;
const LOCAL_TIMELINE_AUTHOR_COUNT = 26;
const HOME_FEED_SELF_COUNT = 51;
const HOME_FEED_OTHER_COUNT = 2;

async function createPublishedPostViaApi(
  page: Page,
  title: string,
): Promise<void> {
  const response = await withTimedAction(page, "api.create_post", () =>
    page.request.post(`${BASE_URL}/api/create_post`, {
      data: {
        body: `# ${title}\n\nBody for ${title}`,
        format: "markdown",
        slug_override: null,
        publish: true,
      },
    }),
  );
  expect(response.ok()).toBeTruthy();
}

test("authenticated user can create a post through the UI", async ({
  page,
}, testInfo) => {
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");

  await expect(page.locator(".j-topbar h1")).toHaveText("New post");
  await page.fill('textarea[name="body"]', "# Playwright Post\n\n**browser**");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");

  await expect(page.locator(".j-save-summary")).toContainText(
    "Post published.",
  );
  await expect(page.locator(".j-save-summary")).toContainText(
    "Slug: playwright-post",
  );
});

test("authenticated user can create a post with a summary", async ({
  page,
}, testInfo) => {
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");

  await expect(page.locator(".j-topbar h1")).toHaveText("New post");
  await page.fill('textarea[name="body"]', "# Summary Test\n\nBody text");
  await page.fill("#compose-summary", "This is a summary");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");

  await expect(page.locator(".j-save-summary")).toContainText(
    "Post published.",
  );

  const permalinkLink = page.locator('[data-test="permalink-link"]');
  const permalinkHref = await permalinkLink.getAttribute("href");
  expect(permalinkHref).toBeTruthy();

  await goto(page, permalinkHref!);

  await expect(page.locator("article h1")).toHaveText("Summary Test");
  await expect(page.locator("article")).toContainText("This is a summary");
});

test("authenticated user can save a draft through the UI", async ({
  page,
}, testInfo) => {
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");

  await page.fill('textarea[name="body"]', "*draft*");
  await click(page, '.j-seg button:has-text("Org")');
  await page.fill('input[name="slug_override"]', "Draft-Slug");
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-save-summary");

  await expect(page.locator(".j-save-summary")).toContainText("Draft saved.");
  await expect(page.locator(".j-save-summary")).toContainText(
    "Slug: draft-slug",
  );
});

test("published post renders at permalink", async ({ page }, testInfo) => {
  test.slow();
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");
  await page.fill(
    'textarea[name="body"]',
    "# Permalink Story\n\n**hello permalink**",
  );
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");

  const summary = page.locator(".j-save-summary");
  await expect(summary).toContainText("Post published.");

  const slugAttr = await summary
    .locator('[data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(slugAttr).toBeTruthy();

  const previewLink = summary.locator('[data-test="preview-link"]');
  await expect(previewLink).toBeVisible();
  const previewHref = await previewLink.getAttribute("href");
  expect(previewHref).toBeTruthy();

  const permalinkLink = summary.locator('[data-test="permalink-link"]');
  await expect(permalinkLink).toBeVisible();
  const permalinkHref = await permalinkLink.getAttribute("href");
  expect(permalinkHref).toBeTruthy();

  const targetUrl = permalinkHref!;

  await goto(page, targetUrl);

  await expect(page.locator("article h1")).toHaveText("Permalink Story");
  await expect(page.locator(".j-post-body")).toContainText("hello permalink");
});

test("authenticated user can edit a draft post", async ({ page }, testInfo) => {
  test.slow();
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Create a draft; title embedded as # heading
  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# Original Draft\n\noriginal body");
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-save-summary");

  const summary = page.locator(".j-save-summary");
  const postIdMatch = (await summary
    .locator('[data-test="preview-link"]')
    .getAttribute("href"))!.match(/\/draft\/(\d+)\/preview/);
  expect(postIdMatch).toBeTruthy();
  const postId = postIdMatch![1];

  // Navigate to edit page
  await goto(page, `/posts/${postId}/edit`);

  await expect(page.locator(".j-topbar h1")).toHaveText("Edit Post");

  // Update the draft; keep heading to preserve the slug
  await page.fill(
    'textarea[name="body"]',
    "# Original Draft\n\n**edited content**",
  );
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-save-summary");

  await expect(page.locator(".j-save-summary")).toContainText("Draft saved.");
  await expect(page.locator(".j-save-summary")).toContainText(
    "Draft saved.Slug: original-draftPreview draft",
  );
});

test("editing a published post freezes the slug", async ({
  page,
}, testInfo) => {
  test.slow();
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Create and publish a post; title embedded as # heading
  await goto(page, "/posts/new");
  await page.fill(
    'textarea[name="body"]',
    "# Published Article\n\noriginal content",
  );
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");

  const summary = page.locator(".j-save-summary");
  const originalSlug = await summary
    .locator('[data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(originalSlug).toBeTruthy();

  const postIdMatch = (await summary
    .locator('[data-test="preview-link"]')
    .getAttribute("href"))!.match(/\/draft\/(\d+)\/preview/);
  expect(postIdMatch).toBeTruthy();
  const postId = postIdMatch![1];

  // Navigate to edit page
  await goto(page, `/posts/${postId}/edit`);

  // Published post should not have a slug_override input
  await expect(page.locator('input[name="slug_override"]')).not.toBeVisible();

  // Save the published post (body already pre-filled from loaded post; slug stays frozen)
  await click(page, 'button[name="publish"][value="true"]');
  // After save, editor redirects to the permalink page
  await waitForSelector(page, "article h1");
  expect(page.url()).toContain(originalSlug!);
});

test("draft lifecycle: create, view, edit, and publish", async ({
  page,
  context,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  const firstNavigationTimeoutMs = hydrationHeavyFirstNavigationTimeoutMs(
    testInfo,
    12_000,
  );
  await register(page, firstNavigationTimeoutMs);

  await goto(page, "/posts/new");
  await page.fill(
    'textarea[name="body"]',
    "# Lifecycle Draft\n\ninitial draft body",
  );
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-save-summary");

  const summary = page.locator(".j-save-summary");
  const previewHref = await summary
    .locator('[data-test="preview-link"]')
    .getAttribute("href");
  expect(previewHref).toBeTruthy();

  const postIdMatch = previewHref!.match(/\/draft\/(\d+)\/preview/);
  expect(postIdMatch).toBeTruthy();
  const postId = postIdMatch![1];

  await goto(page, "/drafts");
  const initialDraftRow = page.locator("li", { hasText: "Lifecycle Draft" });
  await expect(initialDraftRow).toBeVisible();
  const permalinkHref = await initialDraftRow
    .locator('a:has-text("Permalink")')
    .getAttribute("href");
  expect(permalinkHref).toBeTruthy();
  const permalinkUrl = permalinkHref!;

  await goto(page, `/posts/${postId}/edit`);
  await page.fill(
    'textarea[name="body"]',
    "# Lifecycle Draft\n\nedited draft body",
  );
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-save-summary");

  await goto(page, permalinkUrl);
  await expect(page.locator(".j-post-body")).toContainText("edited draft body");
  await expect(page.locator(".draft-banner")).toContainText(
    "Draft - visible only to you",
  );

  const guestContext = await context.browser()!.newContext();
  const guestPage = await guestContext.newPage();
  await goto(guestPage, permalinkUrl, { timeout: firstNavigationTimeoutMs });
  await expect(guestPage.locator("body")).not.toContainText(
    "edited draft body",
  );
  await guestContext.close();

  await goto(page, "/drafts");
  const draftRow = page.locator("li", { hasText: "Lifecycle Draft" });
  await expect(draftRow).toBeVisible();
  await draftRow.locator('button:has-text("Publish")').click();
  await waitForSelector(page, ".success");
  await expect(page.locator(".success")).toContainText("Post published.");

  await goto(page, permalinkUrl);
  await expect(page.locator(".j-post-body")).toContainText("edited draft body");
  await expect(page.locator(".draft-banner")).toHaveCount(0);
});

test("per-user timeline lists published posts with pagination", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  const perf = createPerfProbe(testInfo, "user_timeline_pagination");
  const firstNavigationTimeoutMs = hydrationHeavyFirstNavigationTimeoutMs(
    testInfo,
    10_000,
  );

  const username = await register(page, firstNavigationTimeoutMs);

  await perf.timed("seed_posts", async () => {
    seedPostsViaTool(
      username,
      TIMELINE_PAGE_SIZE + TIMELINE_OVERFLOW_COUNT,
      "Timeline Post",
    );
  });

  await goto(page, `/~${username}`, { timeout: firstNavigationTimeoutMs });

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
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  const perf = createPerfProbe(testInfo, "home_local_timeline");
  const firstNavigationTimeoutMs = hydrationHeavyFirstNavigationTimeoutMs(
    testInfo,
    10_000,
  );

  await perf.timed("seed_author_one", async () => {
    await register(page, firstNavigationTimeoutMs);
    for (let i = 0; i < LOCAL_TIMELINE_AUTHOR_COUNT; i += 1) {
      await createPublishedPostViaApi(page, `Local Author One ${i}`);
    }
  });

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await perf.timed("seed_author_two", async () => {
    await register(secondPage, firstNavigationTimeoutMs);
    for (let i = 0; i < LOCAL_TIMELINE_AUTHOR_COUNT; i += 1) {
      await createPublishedPostViaApi(secondPage, `Local Author Two ${i}`);
    }
  });

  const guestContext = await browser.newContext();
  const guestPage = await guestContext.newPage();
  await goto(guestPage, "/", { timeout: firstNavigationTimeoutMs });

  // Site title is still the seeded value: admin-site (the only mutator) runs in
  // the serial Playwright project and never overlaps this test.
  await expect(guestPage.locator(".j-topbar h1")).toHaveText("jaunder.local");

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
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  const perf = createPerfProbe(testInfo, "home_authenticated_feed");
  const firstNavigationTimeoutMs = hydrationHeavyFirstNavigationTimeoutMs(
    testInfo,
    10_000,
  );

  await perf.timed("seed_self", async () => {
    await register(page, firstNavigationTimeoutMs);
    for (let i = 0; i < HOME_FEED_SELF_COUNT; i += 1) {
      await createPublishedPostViaApi(page, `Home Feed Mine ${i}`);
    }
  });

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await perf.timed("seed_other", async () => {
    await register(secondPage, firstNavigationTimeoutMs);
    for (let i = 0; i < HOME_FEED_OTHER_COUNT; i += 1) {
      await createPublishedPostViaApi(secondPage, `Home Feed Other ${i}`);
    }
  });

  await goto(page, "/app", { timeout: firstNavigationTimeoutMs });

  await expect(page.locator(".j-topbar h1")).toHaveText("Home");
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
  page,
}, testInfo) => {
  test.slow();
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Create a published post; title embedded as # heading (title input is removed from UI)
  await goto(page, "/posts/new");
  await page.fill(
    'textarea[name="body"]',
    "# Post To Delete\n\nthis will be deleted",
  );
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");

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
  await expect(page.locator(".error")).toContainText("Post not found");

  // Verify excluded from user timeline
  const username = permalinkUrl.match(/\/~([^/]+)\//)?.[1];
  expect(username).toBeTruthy();
  await goto(page, `/~${username}`);
  await expect(page.locator("body")).not.toContainText("Post To Delete");
});

test("inline composer: published post appears in timeline without page reload", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // The /app cockpit must already show the feed with the composer.
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  const initialCount = await page.locator("article.j-post").count();

  await page.fill('.j-composer textarea[name="body"]', "Live refresh test");
  await click(page, '.j-composer button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-composer p.success");

  // The new post should appear without a page reload.
  await expect(page.locator("article.j-post")).toHaveCount(initialCount + 1, {
    timeout: hydrationHeavyTimeoutMs(testInfo, 8_000),
  });
});

test("inline composer: plain body publishes titleless note", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
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

test("inline composer: draft flash is a link to the draft preview URL", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  await page.fill('.j-composer textarea[name="body"]', "Draft flash link test");
  await click(page, '.j-composer button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-composer p.success a");

  const link = page.locator(".j-composer p.success a");
  await expect(link).toContainText("Draft saved!");
  const href = await link.getAttribute("href");
  expect(href).toBeTruthy();
});

test("inline composer: flash clears when user starts typing", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 20_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );
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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");

  await page.fill('textarea[name="body"]', "# Tagged Post\n\ncontent");

  // Add three tags via the TagInput: type and press Enter for each
  for (const tag of ["alpha", "beta", "gamma"]) {
    await page.fill(".j-tag-text", tag);
    await page.keyboard.press("Enter");
    await waitForSelector(page, `.j-tag-chip-label:has-text("#${tag}")`);
  }

  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");
  await expect(page.locator(".j-save-summary")).toContainText(
    "Post published.",
  );

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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Create a published post with two tags via the API
  const res = await page.request.post(`${BASE_URL}/api/create_post`, {
    data: {
      body: "# Chip Nav Post\n\ncontent",
      format: "markdown",
      slug_override: null,
      publish: true,
      tags: ["rustlang", "nix"],
    },
  });
  expect(res.ok()).toBeTruthy();
  const { permalink } = (await res.json()) as { permalink: string };
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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 60_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Use tags unique to this test so cross-test pollution can't affect the
  // /tags/:tag listing checks below.
  const res = await page.request.post(`${BASE_URL}/api/create_post`, {
    data: {
      body: "# Tag Edit Post\n\ncontent",
      format: "markdown",
      slug_override: null,
      publish: true,
      tags: ["xedita", "xeditb", "xeditc"],
    },
  });
  expect(res.ok()).toBeTruthy();
  const { permalink, post_id } = (await res.json()) as {
    permalink: string;
    post_id: number;
  };

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
  await click(page, 'button[name="publish"][value="true"]');

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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Seed the tag corpus with a known tag
  const res = await page.request.post(`${BASE_URL}/api/create_post`, {
    data: {
      body: "seed post",
      format: "markdown",
      slug_override: null,
      publish: true,
      tags: ["rustlang"],
    },
  });
  expect(res.ok()).toBeTruthy();

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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Seed a known tag
  const res = await page.request.post(`${BASE_URL}/api/create_post`, {
    data: {
      body: "seed post",
      format: "markdown",
      slug_override: null,
      publish: true,
      tags: ["kbdnav"],
    },
  });
  expect(res.ok()).toBeTruthy();

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
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  const res = await page.request.post(`${BASE_URL}/api/create_post`, {
    data: {
      body: "seed post",
      format: "markdown",
      slug_override: null,
      publish: true,
      tags: ["esctest"],
    },
  });
  expect(res.ok()).toBeTruthy();

  await goto(page, "/posts/new");
  await page.fill(".j-tag-text", "esc");
  await waitForSelector(page, ".j-tag-suggest");

  await page.keyboard.press("Escape");
  await expect(page.locator(".j-tag-suggest")).toHaveCount(0);
  // No chip should have been added
  await expect(page.locator(".j-tag-chip")).toHaveCount(0);
});

test("TagInput: invalid tag text shows an error", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");
  // "bad tag" has a space — invalid after normalize
  await page.fill(".j-tag-text", "bad tag");
  await page.keyboard.press("Enter");

  await waitForSelector(page, ".j-tag-error");
  await expect(page.locator(".j-tag-error")).toContainText("Invalid tag");
  // No chip should appear
  await expect(page.locator(".j-tag-chip")).toHaveCount(0);
});

test("authenticated user can delete a draft from the drafts page", async ({
  page,
}, testInfo) => {
  test.slow();
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Create a draft; title embedded as # heading (title input is removed from UI)
  await goto(page, "/posts/new");
  await page.fill(
    'textarea[name="body"]',
    "# Draft To Delete\n\ndraft content",
  );
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".j-save-summary");

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

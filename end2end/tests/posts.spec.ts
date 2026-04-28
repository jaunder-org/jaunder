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
      form: {
        body: `# ${title}\n\nBody for ${title}`,
        format: "markdown",
        publish: "true",
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
  await waitForSelector(page, ".success");

  await expect(page.locator(".success")).toContainText("Post published.");
  await expect(page.locator(".success")).toContainText("Slug: playwright-post");
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
  await waitForSelector(page, ".success");

  await expect(page.locator(".success")).toContainText("Draft saved.");
  await expect(page.locator(".success")).toContainText("Slug: draft-slug");
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
  await waitForSelector(page, ".success");

  const success = page.locator(".success");
  await expect(success).toContainText("Post published.");

  const slugAttr = await success
    .locator('[data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(slugAttr).toBeTruthy();

  const previewLink = success.locator('[data-test="preview-link"]');
  await expect(previewLink).toBeVisible();
  const previewHref = await previewLink.getAttribute("href");
  expect(previewHref).toBeTruthy();

  const permalinkLink = success.locator('[data-test="permalink-link"]');
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
  await waitForSelector(page, ".success");

  const success = page.locator(".success");
  const postIdMatch = (await success
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
  await waitForSelector(page, ".success");

  await expect(page.locator(".success")).toContainText("Draft saved.");
  await expect(page.locator(".success")).toContainText(
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
  await waitForSelector(page, ".success");

  const success = page.locator(".success");
  const originalSlug = await success
    .locator('[data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(originalSlug).toBeTruthy();

  const postIdMatch = (await success
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
  await waitForSelector(page, ".success");

  const success = page.locator(".success");
  const previewHref = await success
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
  await waitForSelector(page, ".success");

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
    for (let i = 0; i < TIMELINE_PAGE_SIZE + TIMELINE_OVERFLOW_COUNT; i += 1) {
      await createPublishedPostViaApi(page, `Timeline Post ${i}`);
    }
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

  await expect(guestPage.locator(".j-topbar h1")).toHaveText("jaunder.local");
  await expect(guestPage.locator("article.j-post")).toHaveCount(
    TIMELINE_PAGE_SIZE,
  );

  await click(guestPage, 'button:has-text("Load more")');
  perf.mark("load_more_clicked");
  await expect
    .poll(async () => guestPage.locator("article.j-post").count(), {
      timeout: 10_000,
    })
    .toBeGreaterThan(TIMELINE_PAGE_SIZE);
  perf.mark("assertions_complete");
  await perf.log();

  await guestContext.close();
  await secondContext.close();
});

test("home page shows authenticated home feed with pagination", async ({
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

  await goto(page, "/", { timeout: firstNavigationTimeoutMs });

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
  await waitForSelector(page, ".success");

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

  // Home page must already show the feed with the composer.
  await goto(page, "/");
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
  await goto(page, "/");
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
  await goto(page, "/");
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
  await goto(page, "/");
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
  await goto(page, "/");
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
  await goto(page, "/");
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
  await goto(page, "/");
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
  await waitForSelector(page, ".success");

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

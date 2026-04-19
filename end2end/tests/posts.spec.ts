import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { waitForHydration } from "./hydration";
import { createPerfProbe } from "./perf";

async function goto(
  page: Page,
  url: string,
  options?: Parameters<Page["goto"]>[1],
): Promise<void> {
  await withTimedAction(page, "page.goto", () => page.goto(url, options));
}

async function waitForSelector(
  page: Page,
  selector: string,
  options?: Parameters<Page["waitForSelector"]>[1],
): Promise<void> {
  await withTimedAction(page, "wait.selector", () => {
    if (options === undefined) {
      return page.waitForSelector(selector);
    }
    return page.waitForSelector(selector, options);
  });
}

async function click(page: Page, selector: string): Promise<void> {
  await withTimedAction(page, "ui.click", () => page.click(selector));
}

async function register(page: Page): Promise<string> {
  const username = `postuser${Date.now()}${Math.random().toString(36).slice(2, 8)}`;

  await goto(page, "http://localhost:3000/register");
  await waitForHydration(page);
  await withTimedAction(page, "ui.fill.username", () =>
    page.fill('input[name="username"]', username),
  );
  await withTimedAction(page, "ui.fill.password", () =>
    page.fill('input[name="password"]', "testpassword123"),
  );
  await click(page, 'button[type="submit"]');
  // waitForLoadState("networkidle") resolves before Firefox's location.replace()
  // navigation fires, causing a race with subsequent page.goto() calls.
  // Wait for either the success marker or an explicit server error so we
  // fail fast on misconfiguration instead of burning the full test timeout.
  const outcome = await Promise.race([
    page
      .waitForSelector("a[href='/logout']", { timeout: 10_000 })
      .then(() => "ok"),
    page.waitForSelector(".error", { timeout: 10_000 }).then(() => "error"),
  ]);
  if (outcome == "error") {
    const errorText = (
      await page.locator(".error").first().textContent()
    )?.trim();
    throw new Error(`registration failed: ${errorText ?? "unknown error"}`);
  }

  return username;
}

async function createPublishedPostViaApi(
  page: Page,
  title: string,
): Promise<void> {
  const response = await withTimedAction(page, "api.create_post", () =>
    page.request.post("http://localhost:3000/api/create_post", {
      form: {
        title,
        body: `Body for ${title}`,
        format: "markdown",
        publish: "true",
      },
    }),
  );
  expect(response.ok()).toBeTruthy();
}

test("authenticated user can create a post through the UI", async ({
  page,
}) => {
  await register(page);

  await goto(page, "http://localhost:3000/posts/new");
  await waitForHydration(page);

  await expect(page.locator("h1")).toHaveText("New Post");
  await page.fill('input[name="title"]', "Playwright Post");
  await page.fill('textarea[name="body"]', "**browser**");
  await page.selectOption('select[name="format"]', "markdown");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".success");

  await expect(page.locator(".success")).toContainText("Post published.");
  await expect(page.locator(".success")).toContainText("Slug: playwright-post");
});

test("authenticated user can save a draft through the UI", async ({ page }) => {
  await register(page);

  await goto(page, "http://localhost:3000/posts/new");
  await waitForHydration(page);

  await page.fill('input[name="title"]', "Playwright Draft");
  await page.fill('textarea[name="body"]', "*draft*");
  await page.selectOption('select[name="format"]', "org");
  await page.fill('input[name="slug_override"]', "Draft-Slug");
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".success");

  await expect(page.locator(".success")).toContainText("Draft saved.");
  await expect(page.locator(".success")).toContainText("Slug: draft-slug");
});

test("published post renders at permalink", async ({ page }) => {
  test.slow();
  await register(page);

  await goto(page, "http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Permalink Story");
  await page.fill('textarea[name="body"]', "**hello permalink**");
  await page.selectOption('select[name="format"]', "markdown");
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

  const previewUrl = new URL(previewHref!, "http://localhost:3000").toString();
  await goto(page, previewUrl, { waitUntil: "domcontentloaded" });
  await waitForHydration(page);
  await expect(page.locator(".draft-banner")).toContainText("Draft preview");
  await expect(page.locator("article h1")).toHaveText("Permalink Story");
  await expect(page.locator(".content")).toContainText("hello permalink");

  const targetUrl = new URL(permalinkHref!, "http://localhost:3000").toString();

  await goto(page, targetUrl, { waitUntil: "domcontentloaded" });
  await waitForHydration(page);

  await expect(page.locator("article h1")).toHaveText("Permalink Story");
  await expect(page.locator(".content")).toContainText("hello permalink");
});

test("authenticated user can edit a draft post", async ({ page }) => {
  test.slow();
  await register(page);

  // Create a draft
  await goto(page, "http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Original Draft");
  await page.fill('textarea[name="body"]', "original body");
  await page.selectOption('select[name="format"]', "markdown");
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".success");

  const success = page.locator(".success");
  const postIdMatch = (await success
    .locator('[data-test="preview-link"]')
    .getAttribute("href"))!.match(/\/draft\/(\d+)\/preview/);
  expect(postIdMatch).toBeTruthy();
  const postId = postIdMatch![1];

  // Navigate to edit page
  await goto(page, `http://localhost:3000/posts/${postId}/edit`);
  await waitForHydration(page);

  await expect(page.locator("h1")).toHaveText("Edit Post");

  // Update the draft
  await page.fill('input[name="title"]', "Edited Draft");
  await page.fill('textarea[name="body"]', "**edited content**");
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".success");

  await expect(page.locator(".success")).toContainText("Draft saved.");
  await expect(page.locator(".success")).toContainText(
    "Draft saved.Slug: original-draftPreview draft",
  );
});

test("editing a published post freezes the slug", async ({ page }) => {
  test.slow();
  await register(page);

  // Create and publish a post
  await goto(page, "http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Published Article");
  await page.fill('textarea[name="body"]', "original content");
  await page.selectOption('select[name="format"]', "markdown");
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
  await goto(page, `http://localhost:3000/posts/${postId}/edit`);
  await waitForHydration(page);

  // Published post should not have a slug_override input
  await expect(page.locator('input[name="slug_override"]')).not.toBeVisible();

  // Save the published post
  await page.fill('input[name="title"]', "Updated Article");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".success");

  await expect(page.locator(".success")).toContainText("Post updated.");
  const updatedSlug = await page
    .locator('[data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(updatedSlug).toBe(originalSlug);
});

test("draft lifecycle: create, view, edit, and publish", async ({
  page,
  context,
}) => {
  test.setTimeout(30_000);
  await register(page);

  await goto(page, "http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Lifecycle Draft");
  await page.fill('textarea[name="body"]', "initial draft body");
  await page.selectOption('select[name="format"]', "markdown");
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

  await goto(page, "http://localhost:3000/drafts");
  await waitForHydration(page);
  const initialDraftRow = page.locator("li", { hasText: "Lifecycle Draft" });
  await expect(initialDraftRow).toBeVisible();
  const permalinkHref = await initialDraftRow
    .locator('a:has-text("Permalink")')
    .getAttribute("href");
  expect(permalinkHref).toBeTruthy();
  const permalinkUrl = new URL(
    permalinkHref!,
    "http://localhost:3000",
  ).toString();

  await goto(page, permalinkUrl);
  await waitForHydration(page);
  await expect(page.locator(".draft-banner")).toContainText(
    "Draft - visible only to you",
  );
  await expect(page.locator(".content")).toContainText("initial draft body");

  await goto(page, `http://localhost:3000/posts/${postId}/edit`);
  await waitForHydration(page);
  await page.fill('textarea[name="body"]', "edited draft body");
  await click(page, 'button[name="publish"][value="false"]');
  await waitForSelector(page, ".success");

  await goto(page, permalinkUrl);
  await waitForHydration(page);
  await expect(page.locator(".content")).toContainText("edited draft body");
  await expect(page.locator(".draft-banner")).toContainText(
    "Draft - visible only to you",
  );

  const guestContext = await context.browser()!.newContext();
  const guestPage = await guestContext.newPage();
  await goto(guestPage, permalinkUrl);
  await waitForHydration(guestPage);
  await expect(guestPage.locator("body")).not.toContainText(
    "edited draft body",
  );
  await guestContext.close();

  await goto(page, "http://localhost:3000/drafts");
  await waitForHydration(page);
  const draftRow = page.locator("li", { hasText: "Lifecycle Draft" });
  await expect(draftRow).toBeVisible();
  await draftRow.locator('button:has-text("Publish")').click();
  await waitForSelector(page, ".success");
  await expect(page.locator(".success")).toContainText("Post published.");

  await goto(page, permalinkUrl);
  await waitForHydration(page);
  await expect(page.locator(".content")).toContainText("edited draft body");
  await expect(page.locator(".draft-banner")).toHaveCount(0);
});

test("per-user timeline lists published posts with pagination", async ({
  page,
}, testInfo) => {
  test.setTimeout(20_000);
  const perf = createPerfProbe(testInfo, "user_timeline_pagination");

  perf.mark("register_start");
  const username = await register(page);
  perf.mark("register_done");

  perf.mark("seed_posts_start");
  for (let i = 0; i < 55; i += 1) {
    await createPublishedPostViaApi(page, `Timeline Post ${i}`);
  }
  perf.mark("seed_posts_done");

  perf.mark("goto_timeline_start");
  await goto(page, `http://localhost:3000/~${username}`);
  perf.mark("goto_timeline_done");
  await waitForHydration(page);
  perf.mark("hydration_done");

  await expect(page.locator("h1")).toContainText(`Posts by ${username}`);
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(50, {
    timeout: 10_000,
  });
  await expect(
    page.locator('[data-test="timeline-item"]').first(),
  ).toContainText("Timeline Post 54");

  await click(page, 'button:has-text("Load more")');
  perf.mark("load_more_clicked");
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(55, {
    timeout: 10_000,
  });
  await expect(
    page.locator('[data-test="timeline-item"]').last(),
  ).toContainText("Timeline Post 0", { timeout: 10_000 });
  perf.mark("assertions_complete");
  await perf.log({ username });
});

test("home page shows local timeline for unauthenticated users", async ({
  page,
  browser,
}, testInfo) => {
  test.setTimeout(20_000);
  const perf = createPerfProbe(testInfo, "home_local_timeline");

  perf.mark("seed_author_one_start");
  await register(page);
  for (let i = 0; i < 30; i += 1) {
    await createPublishedPostViaApi(page, `Local Author One ${i}`);
  }
  perf.mark("seed_author_one_done");

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  perf.mark("seed_author_two_start");
  await register(secondPage);
  for (let i = 0; i < 30; i += 1) {
    await createPublishedPostViaApi(secondPage, `Local Author Two ${i}`);
  }
  perf.mark("seed_author_two_done");

  const guestContext = await browser.newContext();
  const guestPage = await guestContext.newPage();
  perf.mark("goto_home_start");
  await goto(guestPage, "http://localhost:3000/");
  perf.mark("goto_home_done");
  await waitForHydration(guestPage);
  perf.mark("hydration_done");

  await expect(guestPage.locator("h2")).toHaveText("Local Timeline", {
    timeout: 10_000,
  });
  await expect(guestPage.locator('[data-test="timeline-item"]')).toHaveCount(
    50,
    {
      timeout: 10_000,
    },
  );

  await click(guestPage, 'button:has-text("Load more")');
  perf.mark("load_more_clicked");
  await expect(guestPage.locator('[data-test="timeline-item"]')).toHaveCount(
    100,
    {
      timeout: 10_000,
    },
  );
  perf.mark("assertions_complete");
  await perf.log();

  await guestContext.close();
  await secondContext.close();
});

test("home page shows authenticated home feed with pagination", async ({
  page,
  browser,
}, testInfo) => {
  test.setTimeout(20_000);
  const perf = createPerfProbe(testInfo, "home_authenticated_feed");

  perf.mark("seed_self_start");
  await register(page);
  for (let i = 0; i < 55; i += 1) {
    await createPublishedPostViaApi(page, `Home Feed Mine ${i}`);
  }
  perf.mark("seed_self_done");

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  perf.mark("seed_other_start");
  await register(secondPage);
  for (let i = 0; i < 5; i += 1) {
    await createPublishedPostViaApi(secondPage, `Home Feed Other ${i}`);
  }
  perf.mark("seed_other_done");

  perf.mark("goto_home_start");
  await goto(page, "http://localhost:3000/");
  perf.mark("goto_home_done");
  await waitForHydration(page);
  perf.mark("hydration_done");

  await expect(page.locator("h2")).toContainText("Your Home Feed", {
    timeout: 10_000,
  });
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(50, {
    timeout: 10_000,
  });
  await expect(
    page.locator('[data-test="timeline-item"]').first(),
  ).toContainText("Home Feed Mine 54");
  await expect(page.locator("body")).not.toContainText("Home Feed Other");

  await click(page, 'button:has-text("Load more")');
  perf.mark("load_more_clicked");
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(55, {
    timeout: 10_000,
  });
  await expect(page.locator("body")).not.toContainText("Home Feed Other");
  perf.mark("assertions_complete");
  await perf.log();

  await secondContext.close();
});

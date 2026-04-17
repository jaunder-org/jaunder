import { test, expect, type Page } from "@playwright/test";

async function waitForHydration(page: Page): Promise<void> {
  await page.waitForSelector("body[data-hydrated]");
}

async function register(page: Page): Promise<string> {
  const username = `postuser${Date.now()}${Math.random().toString(36).slice(2, 8)}`;

  await page.goto("http://localhost:3000/register");
  await waitForHydration(page);
  await page.fill('input[name="username"]', username);
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  // waitForLoadState("networkidle") resolves before Firefox's location.replace()
  // navigation fires, causing a race with subsequent page.goto() calls.  Wait
  // for the logout link instead — it only appears after a successful registration
  // and a full-page reload to the home page with the new session cookie.
  await page.waitForSelector("a[href='/logout']");

  return username;
}

async function createPublishedPostViaApi(
  page: Page,
  title: string,
): Promise<void> {
  const response = await page.request.post(
    "http://localhost:3000/api/create_post",
    {
      form: {
        title,
        body: `Body for ${title}`,
        format: "markdown",
        publish: "true",
      },
    },
  );
  expect(response.ok()).toBeTruthy();
}

test("authenticated user can create a post through the UI", async ({
  page,
}) => {
  await register(page);

  await page.goto("http://localhost:3000/posts/new");
  await waitForHydration(page);

  await expect(page.locator("h1")).toHaveText("New Post");
  await page.fill('input[name="title"]', "Playwright Post");
  await page.fill('textarea[name="body"]', "**browser**");
  await page.selectOption('select[name="format"]', "markdown");
  await page.click('button[name="publish"][value="true"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".success")).toContainText("Post published.");
  await expect(page.locator(".success")).toContainText("Slug: playwright-post");
});

test("authenticated user can save a draft through the UI", async ({ page }) => {
  await register(page);

  await page.goto("http://localhost:3000/posts/new");
  await waitForHydration(page);

  await page.fill('input[name="title"]', "Playwright Draft");
  await page.fill('textarea[name="body"]', "*draft*");
  await page.selectOption('select[name="format"]', "org");
  await page.fill('input[name="slug_override"]', "Draft-Slug");
  await page.click('button[name="publish"][value="false"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".success")).toContainText("Draft saved.");
  await expect(page.locator(".success")).toContainText("Slug: draft-slug");
});

test("published post renders at permalink", async ({ page }) => {
  await register(page);

  await page.goto("http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Permalink Story");
  await page.fill('textarea[name="body"]', "**hello permalink**");
  await page.selectOption('select[name="format"]', "markdown");
  await page.click('button[name="publish"][value="true"]');
  await page.waitForSelector(".success");

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
  await page.goto(previewUrl);
  await waitForHydration(page);
  await expect(page.locator(".draft-banner")).toContainText("Draft preview");
  await expect(page.locator("article h1")).toHaveText("Permalink Story");
  await expect(page.locator(".content")).toContainText("hello permalink");

  const targetUrl = new URL(permalinkHref!, "http://localhost:3000").toString();

  await page.goto(targetUrl);
  await waitForHydration(page);

  await expect(page.locator("article h1")).toHaveText("Permalink Story");
  await expect(page.locator(".content")).toContainText("hello permalink");
});

test("authenticated user can edit a draft post", async ({ page }) => {
  await register(page);

  // Create a draft
  await page.goto("http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Original Draft");
  await page.fill('textarea[name="body"]', "original body");
  await page.selectOption('select[name="format"]', "markdown");
  await page.click('button[name="publish"][value="false"]');
  await page.waitForSelector(".success");

  const success = page.locator(".success");
  const postIdMatch = (await success
    .locator('[data-test="preview-link"]')
    .getAttribute("href"))!.match(/\/draft\/(\d+)\/preview/);
  expect(postIdMatch).toBeTruthy();
  const postId = postIdMatch![1];

  // Navigate to edit page
  await page.goto(`http://localhost:3000/posts/${postId}/edit`);
  await waitForHydration(page);

  await expect(page.locator("h1")).toHaveText("Edit Post");

  // Update the draft
  await page.fill('input[name="title"]', "Edited Draft");
  await page.fill('textarea[name="body"]', "**edited content**");
  await page.click('button[name="publish"][value="false"]');
  await page.waitForLoadState("networkidle");

  await expect(page.locator(".success")).toContainText("Draft saved.");
  await expect(page.locator(".success")).toContainText(
    "Draft saved.Slug: original-draftPreview draft",
  );
});

test("editing a published post freezes the slug", async ({ page }) => {
  await register(page);

  // Create and publish a post
  await page.goto("http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Published Article");
  await page.fill('textarea[name="body"]', "original content");
  await page.selectOption('select[name="format"]', "markdown");
  await page.click('button[name="publish"][value="true"]');
  await page.waitForSelector(".success");

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
  await page.goto(`http://localhost:3000/posts/${postId}/edit`);
  await waitForHydration(page);

  // Published post should not have a slug_override input
  await expect(page.locator('input[name="slug_override"]')).not.toBeVisible();

  // Save the published post
  await page.fill('input[name="title"]', "Updated Article");
  await page.click('button[name="publish"][value="true"]');
  await page.waitForLoadState("networkidle");

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

  await page.goto("http://localhost:3000/posts/new");
  await waitForHydration(page);
  await page.fill('input[name="title"]', "Lifecycle Draft");
  await page.fill('textarea[name="body"]', "initial draft body");
  await page.selectOption('select[name="format"]', "markdown");
  await page.click('button[name="publish"][value="false"]');
  await page.waitForSelector(".success");

  const success = page.locator(".success");
  const previewHref = await success
    .locator('[data-test="preview-link"]')
    .getAttribute("href");
  expect(previewHref).toBeTruthy();

  const postIdMatch = previewHref!.match(/\/draft\/(\d+)\/preview/);
  expect(postIdMatch).toBeTruthy();
  const postId = postIdMatch![1];

  await page.goto("http://localhost:3000/drafts");
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

  await page.goto(permalinkUrl);
  await waitForHydration(page);
  await expect(page.locator(".draft-banner")).toContainText(
    "Draft - visible only to you",
  );
  await expect(page.locator(".content")).toContainText("initial draft body");

  await page.goto(`http://localhost:3000/posts/${postId}/edit`);
  await waitForHydration(page);
  await page.fill('textarea[name="body"]', "edited draft body");
  await page.click('button[name="publish"][value="false"]');
  await page.waitForSelector(".success");

  await page.goto(permalinkUrl);
  await waitForHydration(page);
  await expect(page.locator(".content")).toContainText("edited draft body");
  await expect(page.locator(".draft-banner")).toContainText(
    "Draft - visible only to you",
  );

  const guestContext = await context.browser()!.newContext();
  const guestPage = await guestContext.newPage();
  await guestPage.goto(permalinkUrl);
  await waitForHydration(guestPage);
  await expect(guestPage.locator("body")).not.toContainText(
    "edited draft body",
  );
  await guestContext.close();

  await page.goto("http://localhost:3000/drafts");
  await waitForHydration(page);
  const draftRow = page.locator("li", { hasText: "Lifecycle Draft" });
  await expect(draftRow).toBeVisible();
  await draftRow.locator('button:has-text("Publish")').click();
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".success")).toContainText("Post published.");

  await page.goto(permalinkUrl);
  await waitForHydration(page);
  await expect(page.locator(".content")).toContainText("edited draft body");
  await expect(page.locator(".draft-banner")).toHaveCount(0);
});

test("per-user timeline lists published posts with pagination", async ({
  page,
}) => {
  test.setTimeout(60_000);
  const username = await register(page);

  for (let i = 0; i < 55; i += 1) {
    await createPublishedPostViaApi(page, `Timeline Post ${i}`);
  }

  await page.goto(`http://localhost:3000/~${username}`);
  await waitForHydration(page);

  await expect(page.locator("h1")).toContainText(`Posts by ${username}`);
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(50, {
    timeout: 20_000,
  });
  await expect(
    page.locator('[data-test="timeline-item"]').first(),
  ).toContainText("Timeline Post 54");

  await page.click('button:has-text("Load more")');
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(55, {
    timeout: 20_000,
  });
  await expect(
    page.locator('[data-test="timeline-item"]').last(),
  ).toContainText("Timeline Post 0", { timeout: 20_000 });
});

test("home page shows local timeline for unauthenticated users", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  await register(page);
  for (let i = 0; i < 30; i += 1) {
    await createPublishedPostViaApi(page, `Local Author One ${i}`);
  }

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await register(secondPage);
  for (let i = 0; i < 30; i += 1) {
    await createPublishedPostViaApi(secondPage, `Local Author Two ${i}`);
  }

  const guestContext = await browser.newContext();
  const guestPage = await guestContext.newPage();
  await guestPage.goto("http://localhost:3000/");
  await waitForHydration(guestPage);

  await expect(guestPage.locator("h2")).toHaveText("Local Timeline", {
    timeout: 20_000,
  });
  await expect(guestPage.locator('[data-test="timeline-item"]')).toHaveCount(
    50,
    {
      timeout: 20_000,
    },
  );

  await guestPage.click('button:has-text("Load more")');
  await expect(guestPage.locator('[data-test="timeline-item"]')).toHaveCount(
    100,
    {
      timeout: 20_000,
    },
  );

  await guestContext.close();
  await secondContext.close();
});

test("home page shows authenticated home feed with pagination", async ({
  page,
  browser,
}) => {
  test.setTimeout(60_000);
  await register(page);
  for (let i = 0; i < 55; i += 1) {
    await createPublishedPostViaApi(page, `Home Feed Mine ${i}`);
  }

  const secondContext = await browser.newContext();
  const secondPage = await secondContext.newPage();
  await register(secondPage);
  for (let i = 0; i < 5; i += 1) {
    await createPublishedPostViaApi(secondPage, `Home Feed Other ${i}`);
  }

  await page.goto("http://localhost:3000/");
  await waitForHydration(page);

  await expect(page.locator("h2")).toContainText("Your Home Feed", {
    timeout: 20_000,
  });
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(50, {
    timeout: 20_000,
  });
  await expect(
    page.locator('[data-test="timeline-item"]').first(),
  ).toContainText("Home Feed Mine 54");
  await expect(page.locator("body")).not.toContainText("Home Feed Other");

  await page.click('button:has-text("Load more")');
  await expect(page.locator('[data-test="timeline-item"]')).toHaveCount(55, {
    timeout: 20_000,
  });
  await expect(page.locator("body")).not.toContainText("Home Feed Other");

  await secondContext.close();
});

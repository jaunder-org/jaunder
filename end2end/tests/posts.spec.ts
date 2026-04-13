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
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".error")).not.toBeVisible();

  return username;
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

  const permalinkLink = success.locator('[data-test="permalink-link"]');
  await expect(permalinkLink).toBeVisible();
  const permalinkHref = await permalinkLink.getAttribute("href");
  expect(permalinkHref).toBeTruthy();

  const targetUrl = new URL(permalinkHref!, "http://localhost:3000").toString();

  await page.goto(targetUrl);
  await waitForHydration(page);

  await expect(page.locator("article h1")).toHaveText("Permalink Story");
  await expect(page.locator(".content")).toContainText("hello permalink");
});

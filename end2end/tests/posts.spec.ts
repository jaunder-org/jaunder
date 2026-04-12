import { test, expect, type Page } from "@playwright/test";

async function waitForHydration(page: Page): Promise<void> {
  await page.waitForSelector("body[data-hydrated]");
}

async function register(page: Page): Promise<string> {
  const username = `postuser${Date.now()}`;

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

  await expect(page.locator(".success")).toHaveText("Post published.");
  await expect(page.locator("body")).toContainText("Slug: playwright-post");
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

  await expect(page.locator(".success")).toHaveText("Draft saved.");
  await expect(page.locator("body")).toContainText("Slug: draft-slug");
});

import { test, expect, type Page } from "@playwright/test";

async function waitForHydration(page: Page): Promise<void> {
  await page.waitForSelector("body[data-hydrated]");
}

async function register(page: Page): Promise<void> {
  const username = `postuser${Date.now()}`;

  await page.goto("http://localhost:3000/register");
  await waitForHydration(page);
  await page.fill('input[name="username"]', username);
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".error")).not.toBeVisible();
}

test("authenticated user can create a post through the server function", async ({
  page,
}) => {
  await register(page);

  const result = await page.evaluate(async () => {
    const response = await fetch("/api/create_post", {
      method: "POST",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded",
      },
      body: new URLSearchParams({
        title: "Playwright Post",
        body: "**browser**",
        format: "markdown",
        publish: "true",
      }),
    });

    return {
      status: response.status,
      text: await response.text(),
    };
  });

  expect(result.status).toBe(200);
  const created = JSON.parse(result.text);
  expect(created.slug).toBe("playwright-post");
  expect(created.published_at).not.toBeNull();
});

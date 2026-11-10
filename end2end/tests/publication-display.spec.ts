import type { Page } from "@playwright/test";
import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { BASE_URL, click, goto, signInAsNewUser } from "./helpers";
import { createPostViaApi } from "./posts";
import { navigateInApp } from "./navigate";

// Presentation must follow the saved zone, not the viewer's Tokyo device zone.
test.use({ timezoneId: "Asia/Tokyo", viewport: { width: 1280, height: 900 } });

async function selectTheme(page: Page, theme: string): Promise<void> {
  const response = await page.request.post(`${BASE_URL}/api/themes/select`, {
    form: {
      scope: "author",
      "selection[kind]": "built_in",
      "selection[value]": theme,
    },
  });
  expect(response.ok(), await response.text()).toBeTruthy();
}

for (const theme of ["studio", "terminal", "reader"]) {
  test(`saved-zone card and permalink publication in ${theme}`, async ({
    page,
  }, testInfo) => {
    const username = await signInAsNewUser(page);
    await selectTheme(page, theme);
    await createPostViaApi(page, {
      body: "# Publication winter\n\nWinter publication",
      publishAt: "2024-01-01T02:30:00Z",
      timezone: "US/Eastern",
    });
    const summer = await createPostViaApi(page, {
      body: "# Publication summer\n\nSummer publication",
      publishAt: "2024-07-01T02:30:00.123456Z",
      timezone: "US/Eastern",
    });
    expect(summer.permalink).toContain("/2024/07/01/");
    await goto(page, `/~${username}`, {
      timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
    });
    await expect(page.locator("article.j-post")).toHaveCount(2);
    await page.screenshot({ path: testInfo.outputPath(`${theme}-cards.png`) });
    const labels = await page.locator(".j-post-time").allTextContents();
    const projected = await page.request.get(`${BASE_URL}/~${username}`);
    const projectedHtml = await projected.text();
    await navigateInApp(
      page,
      () => page.locator(`a[href="${summer.permalink}"]`).first().click(),
      {
        url: summer.permalink,
        ready: '.j-topbar h1:has-text("Post by")',
      },
    );
    await page.screenshot({
      path: testInfo.outputPath(`${theme}-permalink.png`),
    });
    expect(labels).toEqual(["2024-06-30 22:30 EDT", "2023-12-31 21:30 EST"]);
    expect(projectedHtml).toContain("2024-06-30 22:30 EDT");
    expect(projectedHtml).toContain("2023-12-31 21:30 EST");
    await expect(page.locator(".j-post-time")).toHaveText(
      "2024-06-30 22:30 EDT",
    );
  });
}

test("saved-zone scheduled list and editor notice", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: "# Scheduled publication\n\nFuture publication",
    publishAt: "2037-07-01T02:30:00Z",
    timezone: "US/Eastern",
  });
  await goto(page, "/scheduled", {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
  });
  await expect(page.locator('[data-test="scheduled-row"]')).toHaveCount(1);
  await page.screenshot({ path: testInfo.outputPath("scheduled-list.png") });
  const listTime = await page
    .locator('[data-test="scheduled-go-live"]')
    .textContent();
  await navigateInApp(
    page,
    () => click(page, '[data-test="scheduled-edit-link"]'),
    {
      url: `/posts/${post.post_id}/edit`,
      ready: 'textarea[name="body"]',
    },
  );
  await page.screenshot({ path: testInfo.outputPath("scheduled-editor.png") });
  expect(listTime).toBe("2037-06-30 22:30 EDT");
  await expect(
    page.getByText("Scheduled for 2037-06-30 22:30 EDT", { exact: true }),
  ).toBeVisible();
});

test("saved-zone scheduled permalink notice", async ({ page }, testInfo) => {
  await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: "# Scheduled publication\n\nFuture publication",
    publishAt: "2037-07-01T02:30:00Z",
    timezone: "US/Eastern",
  });
  await goto(page, post.permalink, {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
  });
  await expect(page.locator(`#j-post-${post.post_id}`)).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("scheduled-permalink.png"),
  });
  await expect(
    page.getByText("Scheduled for 2037-06-30 22:30 EDT", { exact: true }),
  ).toBeVisible();
  await expect(page.locator(".j-post-time")).toHaveText("2037-06-30 22:30 EDT");
});

test("current and prior revision publication use their own saved zones", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  const body = "# Publication history\n\nUnchanged publication instant";
  const post = await createPostViaApi(page, {
    body,
    publishAt: "2024-07-01T02:30:00.123456Z",
    timezone: "US/Eastern",
  });
  const changed = await page.request.post(`${BASE_URL}/api/posts/update`, {
    data: {
      post_id: post.post_id,
      post: {
        body,
        format: "markdown",
        publish: true,
        timezone: "Europe/London",
        audience: { public: true, subscribers: false, named: [] },
      },
    },
  });
  expect(changed.ok(), await changed.text()).toBeTruthy();
  await goto(page, `/posts/${post.post_id}/history`, {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
  });
  await expect(page.locator('[data-test="history-current"]')).toBeVisible();
  await expect(page.locator('[data-test="history-row"]')).toHaveCount(1);
  await page.screenshot({ path: testInfo.outputPath("history-current.png") });
  const current = await page
    .locator('[data-test="history-current"]')
    .textContent();
  const detail = page.locator('[data-test="history-detail-link"]').first();
  const detailHref = await detail.getAttribute("href");
  expect(detailHref).toBeTruthy();
  await navigateInApp(page, () => detail.click(), {
    url: detailHref!,
    ready: '[data-test="history-source"]',
  });
  await page.screenshot({ path: testInfo.outputPath("history-revision.png") });
  expect(current).toContain("2024-07-01 03:30 BST");
  await expect(page.locator('[data-test="history-detail-page"]')).toContainText(
    "2024-06-30 22:30 EDT",
  );
  await expect(page.locator('[data-test="history-source"]')).toContainText(
    "Unchanged publication instant",
  );
});

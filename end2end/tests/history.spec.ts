import type { Page } from "@playwright/test";
import { expect, setTestBudget, test } from "./fixtures";
import { BASE_URL, click, goto, waitForSelector } from "./helpers";
import { navigateInApp } from "./navigate";
import { withTimedAction } from "./actions";
import { createPostViaApi } from "./posts";
import { applySeededSession } from "./seed";

const REVISION_PAGE_SIZE = 50;

async function updatePost(
  page: Page,
  postId: number,
  body: string,
): Promise<void> {
  const response = await page.request.post(`${BASE_URL}/api/posts/update`, {
    data: {
      post_id: postId,
      post: {
        body,
        format: "markdown",
        slug_override: null,
        publish: true,
      },
    },
  });
  expect(
    response.ok(),
    `posts::update failed (${response.status()}): ${await response.text()}`,
  ).toBeTruthy();
}

test("owner inspects paginated immutable history and a Deleted Post", async ({
  page,
  user,
  firstNav,
}) => {
  setTestBudget(120_000);
  await applySeededSession(page.context(), user);
  const created = await createPostViaApi(page, {
    body: "# Revision source 0\n\nInitial immutable body",
  });

  await withTimedAction(
    page,
    "api.posts.update.revision_sequence",
    async () => {
      for (
        let revision = 1;
        revision <= REVISION_PAGE_SIZE + 1;
        revision += 1
      ) {
        await updatePost(
          page,
          created.post_id,
          `# Revision source ${revision}\n\nImmutable body ${revision}`,
        );
      }
      // The same canonical full state is a semantic no-op: it must not add a row.
      await updatePost(
        page,
        created.post_id,
        `# Revision source ${REVISION_PAGE_SIZE + 1}\n\nImmutable body ${REVISION_PAGE_SIZE + 1}`,
      );
    },
  );

  await goto(page, created.permalink, { timeout: firstNav });
  await waitForSelector(page, '[data-test="post-history-link"]');
  await navigateInApp(
    page,
    () => click(page, '[data-test="post-history-link"]'),
    {
      url: `/posts/${created.post_id}/history`,
      ready: '[data-test="post-history-page"] [data-test="history-current"]',
    },
  );
  await expect(
    page.locator('[data-test="history-current-lifecycle"]'),
  ).toHaveText("Published");
  await expect(page.locator('[data-test="history-row"]')).toHaveCount(
    REVISION_PAGE_SIZE,
  );
  await click(page, '[data-test="history-load-more"]');
  await expect(page.locator('[data-test="history-row"]')).toHaveCount(
    REVISION_PAGE_SIZE + 1,
  );

  const newestDetailHref = await page
    .locator('[data-test="history-detail-link"]')
    .first()
    .getAttribute("href");
  expect(newestDetailHref).toBeTruthy();
  await navigateInApp(
    page,
    () => click(page, 'tbody tr:first-child [data-test="history-detail-link"]'),
    {
      url: newestDetailHref!,
      ready: '[data-test="history-detail-page"] [data-test="history-source"]',
    },
  );
  await expect(page.locator('[data-test="history-source"]')).toContainText(
    "Immutable body 50",
  );
  await expect(page.locator('[data-test="history-rendered"] h1')).toHaveText(
    "Revision source 50",
  );
  await expect(page.locator('[data-test="history-tags"]')).toContainText(
    "No tags in this snapshot.",
  );
  await expect(page.locator('[data-test="history-audiences"]')).toContainText(
    "public",
  );
  await expect(page.locator('[data-test="history-media"]')).toContainText(
    "No media references in this snapshot.",
  );

  await navigateInApp(
    page,
    () => click(page, '[data-test="history-nav-link"]'),
    {
      url: "/history",
      ready: '[data-test="history-page"] [data-test="history-list"]',
    },
  );
  await expect(page.locator('[data-test="history-row"]')).toHaveCount(
    REVISION_PAGE_SIZE,
  );
  await click(page, '[data-test="history-load-more"]');
  await expect(page.locator('[data-test="history-row"]')).toHaveCount(
    REVISION_PAGE_SIZE + 1,
  );
  await expect(page.locator('[data-test="history-load-more"]')).toHaveCount(0);

  await withTimedAction(page, "api.posts.delete.history_subject", async () => {
    const response = await page.request.post(`${BASE_URL}/api/posts/delete`, {
      form: { post_id: String(created.post_id) },
    });
    expect(
      response.ok(),
      `posts::delete failed (${response.status()}): ${await response.text()}`,
    ).toBeTruthy();
  });

  await navigateInApp(
    page,
    () =>
      click(
        page,
        `tbody tr:first-child a[href="/posts/${created.post_id}/history"]`,
      ),
    {
      url: `/posts/${created.post_id}/history`,
      ready: '[data-test="post-history-page"] [data-test="history-current"]',
    },
  );
  await expect(
    page.locator('[data-test="history-current-lifecycle"]'),
  ).toHaveText("Deleted");

  await navigateInApp(
    page,
    () => click(page, '[data-test="history-nav-link"]'),
    {
      url: "/history",
      ready: '[data-test="history-page"] [data-current-deleted="true"]',
    },
  );
  const deletedRow = page.locator('[data-test="history-row"]').first();
  await expect(deletedRow).toContainText("Deleted");
  const deletedDetailHref = await deletedRow
    .locator('[data-test="history-detail-link"]')
    .getAttribute("href");
  expect(deletedDetailHref).toBeTruthy();
  await navigateInApp(
    page,
    () => click(page, 'tbody tr:first-child [data-test="history-detail-link"]'),
    {
      url: deletedDetailHref!,
      ready: '[data-test="history-detail-page"] [data-test="history-rendered"]',
    },
  );
  await expect(page.locator('[data-test="history-source"]')).toContainText(
    `Immutable body ${REVISION_PAGE_SIZE + 1}`,
  );
});

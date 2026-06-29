import {
  test,
  expect,
  hydrationHeavyFirstNavigationTimeoutMs,
} from "./fixtures";
import { goto, click, waitForSelector, register } from "./helpers";

// Acceptance test for issue #72: slug generation is Unicode-preserving and
// never-fail. A Unicode title round-trips to its (percent-encoded) permalink,
// and a title with no usable characters falls back to the `post` slug. Both
// run against SQLite and Postgres (the e2e-sqlite / e2e-postgres VMs).

test("a Unicode-titled post is reachable at its permalink", async ({
  page,
}, testInfo) => {
  test.slow();
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# Café 日本語\n\nunicode body");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");
  await expect(page.locator(".j-save-summary")).toContainText(
    "Post published.",
  );

  // The server generated the slug; read it rather than constructing it.
  const slug = await page
    .locator('.j-save-summary [data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(slug).toBe("café-日本語");

  const href = await page
    .locator('.j-save-summary [data-test="permalink-link"]')
    .getAttribute("href");
  expect(href).toBeTruthy();

  await goto(page, href!); // the browser percent-encodes the Unicode path segment
  await expect(page.locator("article h1")).toContainText("Café 日本語");
  await expect(page.locator(".j-post-body")).toContainText("unicode body");
});

test("an emoji-only title falls back to the 'post' slug and is reachable", async ({
  page,
}, testInfo) => {
  test.slow();
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# 🚀🎉\n\nemoji body");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");
  await expect(page.locator(".j-save-summary")).toContainText(
    "Post published.",
  );

  const slug = await page
    .locator('.j-save-summary [data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(slug).toBe("post");

  const href = await page
    .locator('.j-save-summary [data-test="permalink-link"]')
    .getAttribute("href");
  await goto(page, href!);
  await expect(page.locator(".j-post-body")).toContainText("emoji body");
});

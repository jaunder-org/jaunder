import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { goto } from "./helpers";
import { composePost, createPostViaApi, followPermalink } from "./posts";
import { applySeededSession, createSessionViaTool } from "./seed";

const source = `#+TITLE: Footnote navigation

First reference[fn:one] and second reference[fn:two]. Again[fn:one].

[fn:one] A [[https://example.org][linked source]] with /emphasis/
continued on a second line.

[fn:two] Another note.`;

test("published Org footnotes link to their notes and back", async ({
  tracedContext,
}, testInfo) => {
  const session = await createSessionViaTool("testlogin");
  const context = await tracedContext();
  await applySeededSession(context, session);
  const page = await context.newPage();
  await goto(page, "/posts/new", {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000),
  });
  const summary = await composePost(page, {
    body: source,
    format: "org",
    audience: "public",
    publish: true,
  });
  await followPermalink(page, summary);
  const body = page.locator("article.j-post .j-post-body");
  await expect(body).toBeVisible();
  if (process.env.JAUNDER_ORG_FOOTNOTE_VISUAL_PROOF) {
    await page.screenshot({
      path: process.env.JAUNDER_ORG_FOOTNOTE_VISUAL_PROOF,
      mask: [page.locator("article.j-post time")],
    });
  }
  const first = body.locator('a[href$="-fn-1"]').first();
  const firstTarget = await first.getAttribute("href");
  expect(firstTarget).toBeTruthy();
  await first.click();
  await expect(body.locator(firstTarget!)).toContainText("linked source");
  await expect(body.locator(firstTarget!)).toContainText(
    "continued on a second line",
  );
  await expect(
    body.locator(firstTarget!).locator('a[href^="#post-"]'),
  ).toHaveCount(2);
  await expect(body.locator('a[href$="-fn-2"]').first()).toBeVisible();
});

test("identical Org footnotes navigate independently on the Home timeline", async ({
  tracedContext,
}, testInfo) => {
  const session = await createSessionViaTool("testlogin");
  const context = await tracedContext();
  await applySeededSession(context, session);
  const page = await context.newPage();
  const first = await createPostViaApi(page, { body: source, format: "org" });
  const second = await createPostViaApi(page, { body: source, format: "org" });
  await goto(page, "/app", {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000),
  });
  expect(first.post_id).not.toBe(second.post_id);
  for (const id of [first.post_id, second.post_id]) {
    const article = page.locator("article.j-post", {
      has: page.locator(`#post-${id}-fn-1`),
    });
    await expect(article).toBeVisible();
    await article.locator(`#post-${id}-fnref-1-1 a`).click();
    await expect(article.locator(`#post-${id}-fn-1`)).toContainText(
      "linked source",
    );
    await expect(
      article.locator(`a[href="#post-${id}-fnref-1-1"]`),
    ).toBeVisible();
  }
});

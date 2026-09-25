import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { goto } from "./helpers";
import { composePost, followPermalink } from "./posts";
import { applySeededSession, createSessionViaTool } from "./seed";

const source = `#+TITLE: Three---two--one...

A long---pause, a short--pause, and then... rest.`;

test("Org special strings render in the public Post title and prose", async ({
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
  await expect(page.locator("article.j-post .j-post-body")).toBeVisible();
  if (process.env.JAUNDER_ORG_VISUAL_PROOF) {
    await page.screenshot({
      path: process.env.JAUNDER_ORG_VISUAL_PROOF,
      mask: [page.locator("article.j-post time")],
    });
  }
  await expect(page.locator("article.j-post .j-post-title")).toHaveText(
    "Three—two–one…",
  );
  await expect(page.locator("article.j-post .j-post-body")).toContainText(
    "A long—pause, a short–pause, and then… rest.",
  );
});

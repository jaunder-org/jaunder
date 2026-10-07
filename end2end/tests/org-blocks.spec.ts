import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { goto } from "./helpers";
import { createPostViaApi } from "./posts";
import { applySeededSession, createSessionViaTool } from "./seed";

const source = `#+TITLE: Verse and quotation

#+begin_verse
To bait fish withal.
  A *bold* second line.
    A [[https://example.org][linked]] third line.

Last line.
#+end_verse

#+begin_quote
First quoted paragraph with /emphasis/.

Second quoted paragraph.
#+end_quote`;

for (const width of [390, 1280]) {
  for (const surface of ["Local", "permalink"]) {
    test(`published Org verse and quotes on ${surface} at ${width}px`, async ({
      tracedContext,
    }, testInfo) => {
      const owner = await tracedContext();
      await applySeededSession(owner, await createSessionViaTool("testlogin"));
      const creator = await owner.newPage();
      const post = await createPostViaApi(creator, {
        body: source,
        format: "org",
        publishAt: "2020-01-02T12:00:00Z",
      });
      const path = new URL(post.permalink, "http://localhost").pathname;
      const viewer = await tracedContext();
      const page = await viewer.newPage();
      await page.setViewportSize({ width, height: 900 });
      await goto(page, surface === "Local" ? "/" : path, {
        timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000),
      });
      const article =
        surface === "Local"
          ? page
              .locator("article.j-post")
              .filter({ has: page.locator(`a[href="${path}"]`) })
          : page.locator("article.j-post");
      const body = article.locator(".j-post-body");
      await expect(body).toContainText("To bait fish withal.");
      const verse = body.locator(":scope > p").first();
      await expect(verse.locator("br")).toHaveCount(5);
      await expect(verse.locator("b")).toHaveText("bold");
      await expect(verse.locator("a")).toHaveText("linked");
      await expect(body.locator("pre")).toHaveCount(0);
      const positions = await verse.evaluate((element) => {
        const first = document.createRange();
        first.selectNode(element.firstChild!);
        const last = document.createRange();
        last.selectNode(
          Array.from(element.childNodes).find(
            (node) => node.textContent === "Last line.",
          )!,
        );
        const bold = element.querySelector("b")!.getBoundingClientRect();
        const link = element.querySelector("a")!.getBoundingClientRect();
        return {
          firstY: first.getBoundingClientRect().y,
          boldY: bold.y,
          linkY: link.y,
          lastY: last.getBoundingClientRect().y,
          boldX: bold.x,
          linkX: link.x,
        };
      });
      expect(positions.boldY).toBeGreaterThan(positions.firstY);
      expect(positions.linkY).toBeGreaterThan(positions.boldY);
      expect(positions.linkX).toBeGreaterThan(positions.boldX);
      expect(positions.lastY - positions.linkY).toBeGreaterThan(
        positions.linkY - positions.boldY,
      );
      await expect(body.locator("blockquote > p")).toHaveCount(2);
      await expect(body.locator("blockquote > p").first()).toContainText(
        "First quoted paragraph",
      );
      await expect(body.locator("blockquote i")).toHaveText("emphasis");
      await expect(body.locator("blockquote > p").last()).toHaveText(
        "Second quoted paragraph.",
      );
      if (
        surface === "permalink" &&
        process.env.JAUNDER_ORG_BLOCKS_VISUAL_PROOF
      ) {
        await page.screenshot({
          path: `${process.env.JAUNDER_ORG_BLOCKS_VISUAL_PROOF}/after-${width}.png`,
          fullPage: true,
        });
      }
    });
  }
}

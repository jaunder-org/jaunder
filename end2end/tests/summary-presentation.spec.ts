import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import {
  BASE_URL,
  confirmedMutation,
  goto,
  signInAsNewUser,
  type MutationOutcome,
} from "./helpers";
import { createPostViaApi } from "./posts";
import {
  conformanceThemePackage,
  contrastRatio,
  publishAndSelectTheme,
} from "./theme-helpers";

const SUMMARY_HOOK = '[data-jaunder-part="post-summary"]';
const POST_HOOK = '[data-jaunder-part="post"]';
const BODY_HOOK = '[data-jaunder-part="post-body"]';
const TITLE_HOOK = '[data-jaunder-part="post-title"]';

type UpdatePublication = "draft" | "published" | "scheduled";
type UpdatedPost = {
  post: { post_id: number; permalink: string };
  publication: UpdatePublication;
};

async function setSummary(
  page: Parameters<typeof goto>[0],
  postId: number,
  body: string,
  summary: string,
  tags: string[] = [],
) {
  const response = await page.request.post(`${BASE_URL}/api/posts/update`, {
    data: {
      post_id: postId,
      post: {
        body,
        format: "markdown",
        slug_override: null,
        publish: true,
        tags,
        summary,
      },
    },
  });
  expect(response.status()).toBe(200);
  confirmedMutation(
    (await response.json()) as MutationOutcome<UpdatedPost>,
    "posts::update",
  );
}

async function summaryPresentation(root: Locator | Page) {
  const summary = await root.locator(SUMMARY_HOOK).evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      color: style.color,
      fontSize: Number.parseFloat(style.fontSize),
      fontStyle: style.fontStyle,
      lineHeight: Number.parseFloat(style.lineHeight),
      marginBottom: Number.parseFloat(style.marginBottom),
    };
  });
  const body = await root.locator(BODY_HOOK).evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      color: style.color,
      fontSize: Number.parseFloat(style.fontSize),
    };
  });
  return { ...summary, bodyColor: body.color, bodyFontSize: body.fontSize };
}

test("Post summaries retain deck hierarchy across public routes and content states", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  const tag = `summary-${username}`;
  const longSummary =
    "A complete authored summary remains visible and wraps naturally across narrow viewports. ".repeat(
      5,
    );

  const titledBody = "# Summary hierarchy proof\n\nTitled body prose.";
  const titled = await createPostViaApi(page, {
    body: titledBody,
    tags: [tag],
  });
  await setSummary(page, titled.post_id, titledBody, longSummary, [tag]);

  const titlelessBody = "Titleless body prose.";
  const titleless = await createPostViaApi(page, { body: titlelessBody });
  await setSummary(
    page,
    titleless.post_id,
    titlelessBody,
    "The titleless summary keeps the same semantic deck role.",
  );
  await createPostViaApi(page, { body: "Summary absent body proof." });

  const context = await tracedContext();
  try {
    for (const route of [
      "/",
      `/~${username}`,
      `/tags/${tag}`,
      titled.permalink,
    ]) {
      const publicPage = await context.newPage();
      try {
        if (route === "/") {
          await publicPage.setViewportSize({ width: 390, height: 844 });
        }
        await goto(publicPage, route, { timeout: firstNav });
        const titledPost = publicPage.locator(POST_HOOK).filter({
          hasText: "Summary hierarchy proof",
        });
        const summary = titledPost.locator(SUMMARY_HOOK);
        await expect(summary).toHaveText(longSummary);
        await expect(summary).toBeVisible();
        await expect(titledPost.locator(TITLE_HOOK)).toHaveCount(1);
        const presentation = await summaryPresentation(titledPost);
        expect(presentation.fontSize).toBeLessThan(presentation.bodyFontSize);
        expect(presentation.color).not.toBe(presentation.bodyColor);
        expect(presentation.fontStyle).toBe("normal");
        expect(presentation.lineHeight).toBeGreaterThan(presentation.fontSize);
        expect(presentation.marginBottom).toBeGreaterThan(0);

        if (route === "/") {
          const horizontalOverflow = await publicPage.evaluate(
            () =>
              document.documentElement.scrollWidth -
              document.documentElement.clientWidth,
          );
          expect(horizontalOverflow).toBeLessThanOrEqual(0);
          const titlelessPost = publicPage.locator(POST_HOOK).filter({
            hasText: "Titleless body prose.",
          });
          await expect(titlelessPost.locator(TITLE_HOOK)).toHaveCount(0);
          await expect(titlelessPost.locator(SUMMARY_HOOK)).toHaveText(
            "The titleless summary keeps the same semantic deck role.",
          );
          const order = await titlelessPost.evaluate((post) =>
            [
              ...post.querySelectorAll(
                '[data-jaunder-part="post-summary"], [data-jaunder-part="post-body"]',
              ),
            ].map((element) => element.getAttribute("data-jaunder-part")),
          );
          expect(order).toEqual(["post-summary", "post-body"]);

          const absentPost = publicPage.locator(POST_HOOK).filter({
            hasText: "Summary absent body proof.",
          });
          await expect(absentPost.locator(SUMMARY_HOOK)).toHaveCount(0);
          await expect(absentPost.locator(BODY_HOOK)).toBeVisible();
        }
      } finally {
        await publicPage.close();
      }
    }
  } finally {
    await context.close();
  }
});

test("a custom Theme Package can override the summary deck in light and dark modes", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  const body = "# Theme summary proof\n\nTheme body prose.";
  const post = await createPostViaApi(page, { body });
  await setSummary(page, post.post_id, body, "Theme-owned summary deck.");

  const themePackage = conformanceThemePackage();
  themePackage.stylesheet += `
[data-jaunder-part="post-summary"] {
  color: rgb(30, 41, 59);
  font-size: 22px;
  font-style: italic;
}
@media (prefers-color-scheme: dark) {
  [data-jaunder-part="post-summary"] { color: rgb(241, 245, 249); }
}`;
  await publishAndSelectTheme(page, themePackage);

  const context = await tracedContext();
  try {
    const publicPage = await context.newPage();
    await publicPage.emulateMedia({ colorScheme: "light" });
    await goto(publicPage, `/~${username}`, { timeout: firstNav });
    const summary = publicPage.locator(SUMMARY_HOOK);
    await expect(summary).toHaveCSS("font-size", "22px");
    await expect(summary).toHaveCSS("font-style", "italic");
    await expect(summary).toHaveCSS("color", "rgb(30, 41, 59)");

    await publicPage.emulateMedia({ colorScheme: "dark" });
    await expect(summary).toHaveCSS("color", "rgb(241, 245, 249)");
    const colors = await summary.evaluate((element) => ({
      summary: getComputedStyle(element).color,
      surface: getComputedStyle(
        document.querySelector('[data-jaunder-part="main"]')!,
      ).backgroundColor,
    }));
    expect(
      contrastRatio(colors.summary, colors.surface),
    ).toBeGreaterThanOrEqual(4.5);
  } finally {
    await context.close();
  }
});

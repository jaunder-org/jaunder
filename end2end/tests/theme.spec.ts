import type { Locator } from "@playwright/test";

import { allowSecondBoot } from "./bootBudget";
import { test, expect } from "./fixtures";
import { BASE_URL, failServerFn, goto, signInAsNewUser } from "./helpers";
import { navigateInApp } from "./navigate";
import { createPostViaApi } from "./posts";
import {
  applySeededSession,
  createSessionViaTool,
  seedPostsViaTool,
  seedUserViaTool,
} from "./seed";
import { expectVisual } from "./visual";
import { expectAccessible } from "./accessibility";

const ROOT = ".j-root";

async function expectSelection(
  group: Locator,
  selected: string,
  labels = ["Site default", "Terminal", "Studio", "Reader"],
): Promise<void> {
  for (const label of labels) {
    const button = group.getByRole("button", { name: label });
    await expect(button).toHaveAttribute(
      "aria-pressed",
      label === selected ? "true" : "false",
    );
  }
}

// Regression for #22: the reactive data-theme binding on the plain `.j-root`
// element must survive the CSR mount. A leaked Leptos `attr:` directive prefix
// produced a literal `attr:data-theme` attribute, so `.j-root[data-theme=...]`
// stopped matching and no theme token overrides applied after the client booted.
test(
  "issue #22: .j-root keeps a real data-theme after CSR mount",
  { tag: ["@visual", "@accessibility"] },
  async ({ page }) => {
    await seedUserViaTool("visualauthor", "visualpassword123");
    await seedPostsViaTool("visualauthor", 1, "Visual Timeline Post");
    await goto(page, "/"); // public projector home; goto() waits for the CSR mount

    const probe = await page.evaluate(() => {
      const root = document.querySelector(".j-root");
      if (!root) return { found: false } as const;
      return {
        found: true as const,
        dataTheme: root.getAttribute("data-theme"),
        attrNames: Array.from(root.attributes).map((a) => a.name),
        accentInk: getComputedStyle(root)
          .getPropertyValue("--accent-ink")
          .trim(),
      };
    });

    expect(probe.found).toBe(true);
    if (!probe.found) return; // narrow the type for the assertions below

    expect(probe.dataTheme).toBe("studio");
    expect(probe.attrNames.some((n) => n.startsWith("attr:"))).toBe(false);
    expect(probe.accentInk).toBe("#3a2fc9");

    const post = page
      .locator("article.j-post")
      .filter({ hasText: "Visual Timeline Post 0" });
    await expect(post).toBeVisible();
    await expect(post).toContainText("Body for Visual Timeline Post 0");
    await expect(post).toContainText("visualauthor");
    await expectVisual(page, "public-timeline.png", {
      mask: [page.locator(".j-post-time")],
    });
    await expectAccessible(page);
  },
);

test("author theme control persists the override without a browser-local preference", async ({
  tracedContext,
}) => {
  const author = await seedUserViaTool("themeauthor", "themepassword123");
  const context = await tracedContext();
  try {
    await applySeededSession(context, author);
    const page = await context.newPage();
    await goto(page, "/profile");

    const authorTheme = page.getByRole("group", { name: "Your pages theme" });
    await expect(authorTheme).toBeVisible();
    await expect(page.getByRole("group", { name: "Site theme" })).toHaveCount(
      0,
    );
    await expectSelection(authorTheme, "Site default");

    await authorTheme.getByRole("button", { name: "Reader" }).click();
    await expectSelection(authorTheme, "Reader");
  } finally {
    await context.close();
  }

  const freshContext = await tracedContext();
  try {
    await applySeededSession(freshContext, author);
    const freshPage = await freshContext.newPage();
    await goto(freshPage, "/profile");
    await expectSelection(
      freshPage.getByRole("group", { name: "Your pages theme" }),
      "Reader",
    );
  } finally {
    await freshContext.close();
  }
});

test("failed author theme load shows an error without selecting a fallback", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await failServerFn(page, "profile/get_your_pages_theme");
  await goto(page, "/profile");

  const group = page.getByRole("group", { name: "Your pages theme" });
  await expect(group).toBeVisible();
  await expectSelection(group, "", [
    "Site default",
    "Terminal",
    "Studio",
    "Reader",
  ]);
  const error = page
    .locator(".j-card", { hasText: "Your pages theme" })
    .locator("p.error");
  await expect(error).toBeVisible();
  await expect(error).not.toContainText("last confirmed choice");
});

test("site and author themes determine fresh anonymous public presentation", async ({
  tracedContext,
}) => {
  const operator = await createSessionViaTool("testoperator", "theme-operator");
  const author = await seedUserViaTool("themeauthorpublic", "themepassword123");
  const tag = "themeproof";

  const operatorContext = await tracedContext();
  try {
    await applySeededSession(operatorContext, operator);
    const operatorPage = await operatorContext.newPage();
    await goto(operatorPage, "/profile");

    const siteTheme = operatorPage.getByRole("group", { name: "Site theme" });
    const authorTheme = operatorPage.getByRole("group", {
      name: "Your pages theme",
    });
    await expect(siteTheme).toBeVisible();
    await expect(authorTheme).toBeVisible();
    await expectSelection(siteTheme, "Studio", [
      "Terminal",
      "Studio",
      "Reader",
    ]);
    await expectSelection(authorTheme, "Site default");

    await siteTheme.getByRole("button", { name: "Terminal" }).click();
    await expectSelection(siteTheme, "Terminal", [
      "Terminal",
      "Studio",
      "Reader",
    ]);
  } finally {
    await operatorContext.close();
  }

  const authorContext = await tracedContext();
  let permalink: string | undefined;
  try {
    await applySeededSession(authorContext, author);
    const authorPage = await authorContext.newPage();
    await goto(authorPage, "/profile");
    const authorTheme = authorPage.getByRole("group", {
      name: "Your pages theme",
    });
    await expect(
      authorPage.getByRole("group", { name: "Site theme" }),
    ).toHaveCount(0);
    await expectSelection(authorTheme, "Site default");

    await authorTheme.getByRole("button", { name: "Reader" }).click();
    await expectSelection(authorTheme, "Reader");

    ({ permalink } = await createPostViaApi(authorPage, {
      body: "# Theme proof\n\npublic presentation",
      tags: [tag],
    }));
  } finally {
    await authorContext.close();
  }

  if (permalink === undefined) {
    throw new Error("theme proof post did not return a permalink");
  }

  const anonymousContext = await tracedContext();
  expect(await anonymousContext.cookies()).toEqual([]);
  try {
    const anonymousPage = await anonymousContext.newPage();
    const projectedHome = await anonymousPage.request.get(`${BASE_URL}/`);
    await expect(projectedHome).toBeOK();
    await expect(await projectedHome.text()).toContain(
      '<div class="j-root" data-theme="terminal">',
    );

    const projectedTag = await anonymousPage.request.get(
      `${BASE_URL}/tags/${tag}`,
    );
    await expect(projectedTag).toBeOK();
    await expect(await projectedTag.text()).toContain(
      '<div class="j-root" data-theme="terminal">',
    );

    for (const path of [`/~${author.username}`, permalink]) {
      const response = await anonymousPage.request.get(`${BASE_URL}${path}`);
      await expect(response).toBeOK();
      await expect(await response.text()).toContain(
        '<div class="j-root" data-theme="reader">',
      );
    }

    await goto(anonymousPage, "/");
    await expect(anonymousPage.locator(ROOT)).toHaveAttribute(
      "data-theme",
      "terminal",
    );
    expect(await anonymousPage.evaluate(() => localStorage.length)).toBe(0);
    await navigateInApp(
      anonymousPage,
      () => anonymousPage.locator(`a[href="${permalink}"]`).first().click(),
      {
        url: permalink,
        ready: `.j-tag-here[href="/~${author.username}/tags/${tag}"]`,
      },
    );
    await expect(anonymousPage.locator(ROOT)).toHaveAttribute(
      "data-theme",
      "reader",
    );
    await navigateInApp(
      anonymousPage,
      () =>
        anonymousPage
          .locator(`.j-tag-here[href="/~${author.username}/tags/${tag}"]`)
          .click(),
      {
        url: `/~${author.username}/tags/${tag}`,
        ready: ".j-topbar",
      },
    );
    await expect(anonymousPage.locator(ROOT)).toHaveAttribute(
      "data-theme",
      "reader",
    );
    await navigateInApp(
      anonymousPage,
      () => anonymousPage.locator(`a[href="${permalink}"]`).first().click(),
      { url: permalink, ready: ".j-page article.j-post" },
    );
    await expect(anonymousPage.locator(ROOT)).toHaveAttribute(
      "data-theme",
      "reader",
    );
    await navigateInApp(
      anonymousPage,
      () => anonymousPage.locator('.j-nav a[href="/"]').click(),
      { url: "/", ready: ".j-topbar" },
    );
    await navigateInApp(
      anonymousPage,
      () => anonymousPage.locator('a[href="/login"]').click(),
      { url: "/login", ready: 'input[name="username"]' },
    );
    await expect(anonymousPage.locator(ROOT)).toHaveAttribute(
      "data-theme",
      "studio",
    );
  } finally {
    await anonymousContext.close();
  }

  const operatorResetContext = await tracedContext();
  try {
    await applySeededSession(operatorResetContext, operator);
    const operatorPage = await operatorResetContext.newPage();
    await goto(operatorPage, "/profile");
    const siteTheme = operatorPage.getByRole("group", { name: "Site theme" });
    await siteTheme.getByRole("button", { name: "Studio" }).click();
    await expectSelection(siteTheme, "Studio", [
      "Terminal",
      "Studio",
      "Reader",
    ]);
  } finally {
    await operatorResetContext.close();
  }

  const authorResetContext = await tracedContext();
  try {
    await applySeededSession(authorResetContext, author);
    const authorPage = await authorResetContext.newPage();
    await goto(authorPage, "/profile");
    const authorTheme = authorPage.getByRole("group", {
      name: "Your pages theme",
    });
    await authorTheme.getByRole("button", { name: "Site default" }).click();
    await expectSelection(authorTheme, "Site default");
  } finally {
    await authorResetContext.close();
  }

  const inheritedContext = await tracedContext();
  try {
    const inheritedPage = await inheritedContext.newPage();
    await goto(inheritedPage, `/~${author.username}`);
    await expect(inheritedPage.locator(ROOT)).toHaveAttribute(
      "data-theme",
      "studio",
    );
    allowSecondBoot(
      inheritedPage,
      "the site tag has no direct in-app entry after this author timeline assertion",
    );
    await goto(inheritedPage, `/tags/${tag}`);
    await expect(inheritedPage.locator(ROOT)).toHaveAttribute(
      "data-theme",
      "studio",
    );
  } finally {
    await inheritedContext.close();
  }
});

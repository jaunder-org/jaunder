import { test, expect } from "./fixtures";
import { goto, signInAsNewUser } from "./helpers";
import { navigateInApp } from "./navigate";
import {
  resetThemeViaTool,
  seedPostsViaTool,
  seedThemeViaTool,
  seedUserViaTool,
} from "./seed";
import { expectVisual } from "./visual";
import { expectAccessible } from "./accessibility";

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

test("published custom author theme survives cold load and in-app navigation", async ({
  page,
}) => {
  const username = await signInAsNewUser(page);
  await seedPostsViaTool(username, 1, "Visual Theme Navigation");

  const expectCustomPresentation = async () => {
    const presentation = await page.evaluate(() => {
      const root = document.querySelector(".j-root");
      const surface = document.querySelector(
        '[data-jaunder-theme-surface][data-jaunder-style-contract="1"]',
      );
      const stylesheet = document.querySelector(
        "link[data-jaunder-theme-stylesheet]",
      );
      return {
        dataTheme: root?.getAttribute("data-theme"),
        stylesheetHref: stylesheet?.getAttribute("href"),
        ready: surface ? getComputedStyle(surface).outlineColor : "",
      };
    });

    expect(presentation.dataTheme).toBe("custom");
    expect(presentation.stylesheetHref).toMatch(/^\/themes\/[0-9a-f]{64}$/);
    expect(presentation.ready).toBe("rgb(1, 2, 3)");
    await expect(page.locator('[data-jaunder-part="logo"]')).toBeVisible();
    await expect(
      page.locator('[data-jaunder-part="header-image"]'),
    ).toBeVisible();
  };

  try {
    await seedThemeViaTool(username);
    await goto(page, `/~${username}`);
    await expectCustomPresentation();

    await navigateInApp(
      page,
      () =>
        page.evaluate(() => {
          history.pushState({}, "", "/");
          window.dispatchEvent(new PopStateEvent("popstate"));
        }),
      { url: "/", ready: '.j-root[data-theme="studio"]' },
    );
    await expect(page.locator(".j-topbar h1")).toHaveText("jaunder.local");
    await expect(
      page.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(0);
    await expect(page.locator('[data-jaunder-part="logo"]')).toHaveCount(0);
    await expect(
      page.locator('[data-jaunder-part="header-image"]'),
    ).toHaveCount(0);
  } finally {
    await resetThemeViaTool(username);
  }
});

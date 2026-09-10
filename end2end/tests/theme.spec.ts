import { test, expect } from "./fixtures";
import { goto, signInAsNewUser } from "./helpers";
import {
  applySeededSession,
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
    const session = await seedUserViaTool("visualauthor", "visualpassword123");
    await seedPostsViaTool("visualauthor", 2, "Visual Timeline Post");
    await applySeededSession(page.context(), session);
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

    const posts = page
      .locator("article.j-post")
      .filter({ hasText: "Visual Timeline Post" });
    await expect(posts).toHaveCount(2);
    const firstPost = posts.filter({ hasText: "Visual Timeline Post 0" });
    const secondPost = posts.filter({ hasText: "Visual Timeline Post 1" });
    await expect(firstPost).toContainText("Body for Visual Timeline Post 0");
    await expect(secondPost).toContainText("Body for Visual Timeline Post 1");
    await expect(firstPost).toContainText("visualauthor");
    await expect(secondPost).toContainText("visualauthor");
    await expect(page.getByRole("button", { name: "Actions" })).toHaveCount(2);
    await expectVisual(page, "public-timeline.png", {
      mask: [page.locator(".j-post-time")],
    });
    await expectAccessible(page);
  },
);

test("a theme-hidden Post anchor recovers through Studio", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  await seedPostsViaTool(username, 1, "Visual Theme Navigation");

  const expectCustomPresentation = async (target = page) => {
    const presentation = await target.evaluate(() => {
      const root = document.querySelector(".j-root");
      const surface = document.querySelector(
        '[data-jaunder-theme-surface][data-jaunder-style-contract="1"]',
      );
      const stylesheet = document.querySelector(
        "link[data-jaunder-theme-stylesheet]",
      );
      const style = surface ? getComputedStyle(surface) : null;
      const slot = document.querySelector(".j-post-actions-slot");
      const slotStyle = slot ? getComputedStyle(slot) : null;
      return {
        dataTheme: root?.getAttribute("data-theme"),
        stylesheetHref: stylesheet?.getAttribute("href"),
        ready: style?.outlineColor ?? "",
        position: style?.position ?? "",
        inset: style?.inset ?? "",
        zIndex: style?.zIndex ?? "",
        transform: style?.transform ?? "",
        filter: style?.filter ?? "",
        overflow: style?.overflow ?? "",
        slotDisplay: slotStyle?.display ?? "",
        slotPosition: slotStyle?.position ?? "",
        slotBoxSizing: slotStyle?.boxSizing ?? "",
        slotMinWidth: slotStyle?.minWidth ?? "",
        slotWidth: slotStyle?.width ?? "",
        slotMaxWidth: slotStyle?.maxWidth ?? "",
        slotMinHeight: slotStyle?.minHeight ?? "",
        slotHeight: slotStyle?.height ?? "",
        slotMaxHeight: slotStyle?.maxHeight ?? "",
      };
    });

    expect(presentation.dataTheme).toBe("custom");
    expect(presentation.stylesheetHref).toMatch(/^\/theme\/[0-9a-f]{64}$/);
    expect(presentation.ready).toBe("rgb(1, 2, 3)");
    expect(presentation.position).toBe("fixed");
    expect(presentation.inset).toBe("0px");
    expect(presentation.zIndex).toBe("2147483647");
    expect(presentation.transform).not.toBe("none");
    expect(presentation.filter).not.toBe("none");
    expect(presentation.overflow).toBe("visible");
    expect(presentation.slotDisplay).toBe("block");
    expect(presentation.slotPosition).toBe("static");
    expect(presentation.slotBoxSizing).toBe("border-box");
    expect(presentation.slotMinWidth).toBe("72px");
    expect(presentation.slotWidth).toBe("72px");
    expect(presentation.slotMaxWidth).toBe("72px");
    expect(presentation.slotMinHeight).toBe("32px");
    expect(presentation.slotHeight).toBe("32px");
    expect(presentation.slotMaxHeight).toBe("32px");
    await expect(target.locator('[data-jaunder-part="logo"]')).toBeVisible();
    await expect(
      target.locator('[data-jaunder-part="header-image"]'),
    ).toBeVisible();
  };

  try {
    await seedThemeViaTool(username);
    await goto(page, `/~${username}`);
    await expectCustomPresentation();

    // The fixture deliberately moves every Post outside the viewport. Its CSS is
    // scoped to the theme surface, so it cannot directly restyle the trusted
    // sibling; it can only make the Post slot's visual anchor unavailable.
    const trustedActions = page
      .getByRole("button", { name: "Actions" })
      .first();
    await expect(trustedActions).not.toBeInViewport();

    const anonymousContext = await tracedContext();
    try {
      const anonymousPage = await anonymousContext.newPage();
      await goto(anonymousPage, `/~${username}`);
      await expectCustomPresentation(anonymousPage);
      await expect(
        anonymousPage.locator(".j-trusted-post-actions"),
      ).toBeEmpty();
    } finally {
      await anonymousContext.close();
    }

    // The destructive theme covers its own public sidebar, so recovery starts
    // from a fresh `/themes` entry in the same authenticated browser context.
    // `/themes` is always Studio and remains reachable by URL.
    const recoveryPage = await page.context().newPage();
    try {
      await goto(recoveryPage, "/themes", { timeout: firstNav });
      const publicSelection = recoveryPage.getByLabel("Public selection");
      await publicSelection.selectOption("studio");
      await expect(publicSelection).toHaveValue("studio");
    } finally {
      await recoveryPage.close();
    }

    const recoveredPage = await page.context().newPage();
    try {
      await goto(recoveredPage, `/~${username}`, { timeout: firstNav });
      await expect(recoveredPage.locator(".j-root")).toHaveAttribute(
        "data-theme",
        "studio",
      );
      const recoveredActions = recoveredPage
        .getByRole("button", { name: "Actions" })
        .first();
      await expect(recoveredActions).toBeVisible();
      await recoveredActions.click({ trial: true });
    } finally {
      await recoveredPage.close();
    }
  } finally {
    await resetThemeViaTool(username);
  }
});

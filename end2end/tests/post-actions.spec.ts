import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { expectAccessible } from "./accessibility";
import { goto, signInAsNewUser } from "./helpers";
import { createPostViaApi, openPostActions } from "./posts";
import { navigateInApp } from "./navigate";
import {
  conformanceThemePackage,
  publishAndSelectTheme,
} from "./theme-helpers";

async function expectPostActionsTextAlignment(page: Page, post: Locator) {
  const slot = post.locator(".j-post-actions-slot");
  const time = post.locator('[data-jaunder-part="published-time"]');
  await expect(slot).toHaveCount(1);
  await expect(time).toHaveCount(1);

  const anchorName = await slot.evaluate((element) =>
    (element as HTMLElement).style.getPropertyValue("anchor-name"),
  );
  expect(anchorName).toMatch(/^--j-post-actions-\d+$/);
  const controlIndex = await page.locator(".j-post-action-control").evaluateAll(
    (controls, anchor) =>
      controls.findIndex(
        (control) =>
          (
            getComputedStyle(control) as CSSStyleDeclaration & {
              positionAnchor: string;
            }
          ).positionAnchor === anchor,
      ),
    anchorName,
  );
  expect(controlIndex).toBeGreaterThanOrEqual(0);
  const control = page.locator(".j-post-action-control").nth(controlIndex);
  const trigger = control.locator(".j-post-action-trigger");
  await expect(trigger).toHaveText("Actions");

  const boxFor = (element: Locator) =>
    element.evaluate((node) => {
      const box = node.getBoundingClientRect();
      return {
        bottom: box.bottom,
        height: box.height,
        left: box.left,
        right: box.right,
        top: box.top,
        width: box.width,
      };
    });
  const [headerBox, ...geometry] = await Promise.all([
    boxFor(post.locator('[data-jaunder-part="post-header"]')),
    boxFor(slot),
    boxFor(control),
    boxFor(trigger),
  ]);
  for (const box of geometry) {
    expect(box.width).toBe(72);
    expect(box.height).toBe(32);
    expect(box.left).toBe(geometry[0].left);
    expect(box.right).toBe(geometry[0].right);
    expect(box.top).toBe(geometry[0].top);
    expect(box.bottom).toBe(geometry[0].bottom);
    expect(box.left).toBeGreaterThanOrEqual(headerBox.left);
    expect(box.right).toBeLessThanOrEqual(headerBox.right);
    expect(box.top).toBeGreaterThanOrEqual(headerBox.top);
    expect(box.bottom).toBeLessThanOrEqual(headerBox.bottom);
  }
  const viewport = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(geometry[0].left).toBeGreaterThanOrEqual(0);
  expect(geometry[0].right).toBeLessThanOrEqual(viewport.clientWidth);
  expect(viewport.scrollWidth).toBeLessThanOrEqual(viewport.clientWidth);

  const protectedMetrics = async (element: Locator) =>
    element.evaluate((node) => {
      const style = getComputedStyle(node);
      return {
        borderBottomWidth: style.borderBottomWidth,
        borderTopWidth: style.borderTopWidth,
        fontFamily: style.fontFamily,
        fontSize: style.fontSize,
        fontStyle: style.fontStyle,
        fontWeight: style.fontWeight,
        letterSpacing: style.letterSpacing,
        lineHeight: style.lineHeight,
        paddingBottom: style.paddingBottom,
        paddingTop: style.paddingTop,
      };
    });
  const [slotMetrics, triggerMetrics] = await Promise.all([
    protectedMetrics(slot),
    protectedMetrics(trigger),
  ]);
  expect(slotMetrics).toEqual(triggerMetrics);
  const probeProtection = await slot.evaluate((node) => {
    const style = getComputedStyle(node) as CSSStyleDeclaration & {
      webkitTextStrokeWidth: string;
    };
    return {
      color: style.color,
      overflow: style.overflow,
      pointerEvents: style.pointerEvents,
      textIndent: style.textIndent,
      textShadow: style.textShadow,
      webkitTextStrokeWidth: style.webkitTextStrokeWidth,
    };
  });
  expect(probeProtection).toEqual({
    color: "rgba(0, 0, 0, 0)",
    overflow: "hidden",
    pointerEvents: "none",
    textIndent: "-9999px",
    textShadow: "none",
    webkitTextStrokeWidth: "0px",
  });

  const baseline = async (element: Locator) =>
    element.evaluate((node) => {
      node.querySelector('[data-test="post-actions-baseline"]')?.remove();
      (node as HTMLElement).style.setProperty(
        "white-space",
        "nowrap",
        "important",
      );
      const probe = document.createElement("span");
      probe.dataset.test = "post-actions-baseline";
      probe.setAttribute("aria-hidden", "true");
      probe.style.setProperty("all", "initial", "important");
      probe.style.setProperty("display", "inline-block", "important");
      probe.style.setProperty("width", "0", "important");
      probe.style.setProperty("height", "0", "important");
      probe.style.setProperty("margin", "0", "important");
      probe.style.setProperty("padding", "0", "important");
      probe.style.setProperty("border", "0", "important");
      probe.style.setProperty("vertical-align", "baseline", "important");
      node.append(probe);
      return probe.getBoundingClientRect().bottom;
    });
  const [timeBaseline, labelBaseline] = await Promise.all([
    baseline(time),
    baseline(trigger),
  ]);
  expect(
    Math.abs(timeBaseline - labelBaseline),
    JSON.stringify({
      timeBaseline,
      labelBaseline,
      headerBox,
      geometry,
      viewport,
    }),
  ).toBeLessThanOrEqual(1);
}

test("owner Post Actions disclosures use native popover dismissal and focus", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  await createPostViaApi(page, { body: "# First actions probe\n\nFirst" });
  await createPostViaApi(page, { body: "# Second actions probe\n\nSecond" });
  const hostileTheme = conformanceThemePackage();
  hostileTheme.stylesheet += `
[data-jaunder-part="post-header"] {
  --font-body: "Conformance Sans";
  --fs-body: 32px;
  --lh-body: 2.5;
  -webkit-text-stroke: 4px rgb(220, 38, 38);
  text-shadow: 8px 8px rgb(220, 38, 38);
}`;
  await publishAndSelectTheme(page, hostileTheme);
  await goto(page, `/~${username}`, { timeout: firstNav });
  await expect(page.locator(".j-root")).toHaveAttribute("data-theme", "custom");

  // `goto` performs a fresh document entry; this must mount the portalled
  // controls before any later in-app route remount.
  const trustedActions = page.locator(".j-trusted-post-actions");
  await expect(
    trustedActions.getByRole("button", { name: "Actions" }),
  ).toHaveCount(2);

  const triggers = page.getByRole("button", { name: "Actions" });
  await expect(triggers).toHaveCount(2);
  await expectPostActionsTextAlignment(
    page,
    page.locator("article.j-post").first(),
  );

  const firstTrigger = triggers.first();
  const firstPopoverId = await firstTrigger.getAttribute("popovertarget");
  expect(firstPopoverId).toBeTruthy();
  const firstPopover = page.locator(`#${firstPopoverId!}`);

  await openPostActions(page);
  // Native popover invokers expose expanded state through the accessibility tree;
  // browsers need not materialize an `aria-expanded` content attribute.
  await expect(
    page.getByRole("button", { name: "Actions", expanded: true }),
  ).toHaveCount(1);
  await expect(firstPopover).toHaveAttribute("popover", "auto");
  await expect(firstPopover).toBeVisible();
  await expect(firstTrigger).toBeFocused();

  await page.keyboard.press("Tab");
  await expect(firstPopover.getByRole("link", { name: "Edit" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(firstPopover).not.toBeVisible();
  await expect(firstTrigger).toBeFocused();

  await firstTrigger.focus();
  await page.keyboard.press("Enter");
  await expect(firstPopover).toBeVisible();
  await page.locator(".j-topbar h1").click();
  await expect(firstPopover).not.toBeVisible();
  await expect(firstTrigger).toBeFocused();

  await openPostActions(page);
  const secondTrigger = triggers.nth(1);
  await secondTrigger.click();
  await expect(firstPopover).not.toBeVisible();
  await expect(page.locator(":popover-open")).toHaveCount(1);
  // Closing the first native auto popover must not return focus to its invoker
  // after the second trigger has opened its own disclosure.
  await expect(secondTrigger).toBeFocused();

  await page.setViewportSize({ width: 375, height: 800 });
  await page.locator("article.j-post").nth(1).scrollIntoViewIfNeeded();
  await expectPostActionsTextAlignment(
    page,
    page.locator("article.j-post").nth(1),
  );
  await expect(triggers.nth(1)).toBeInViewport();
  const narrowPopover = await page.locator(":popover-open").boundingBox();
  expect(narrowPopover).not.toBeNull();
  expect(narrowPopover!.x).toBeGreaterThanOrEqual(0);
  expect(narrowPopover!.x + narrowPopover!.width).toBeLessThanOrEqual(375);

  // The package remains confined to public presentation; the owner can always
  // recover through the unthemed Studio surface in the same session.
  const studioPage = await page.context().newPage();
  try {
    await goto(studioPage, "/themes", { timeout: firstNav });
    await expect(studioPage.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "studio",
    );
    await expect(
      studioPage.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(0);
  } finally {
    await studioPage.close();
  }

  // Keep accessibility scanning independent from native popover interaction:
  // axe may disturb that state, so no later assertion depends on preserving it.
  await expect(page.locator(":popover-open")).toHaveCount(1);
  await expectAccessible(page);

  const anonymousContext = await tracedContext();
  try {
    const anonymousPage = await anonymousContext.newPage();
    await goto(anonymousPage, `/~${username}`, { timeout: firstNav });
    await expect(
      anonymousPage.getByRole("button", { name: "Actions" }),
    ).toHaveCount(0);
    await expect(anonymousPage.locator(".j-trusted-post-actions")).toBeEmpty();
  } finally {
    await anonymousContext.close();
  }
});

test("owner permalink Post Actions menu stays attached to its trigger", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: "# Permalink actions placement probe\n\nBody",
  });
  await goto(page, post.permalink, { timeout: firstNav });

  const trigger = page.getByRole("button", { name: "Actions" });
  await expect(trigger).toBeVisible();

  for (const viewport of [
    { width: 1280, height: 720 },
    { width: 375, height: 800 },
  ]) {
    await page.setViewportSize(viewport);
    await trigger.scrollIntoViewIfNeeded();
    await expectPostActionsTextAlignment(page, page.locator("article.j-post"));
    const popover = await openPostActions(page);
    await expect(popover.getByRole("link", { name: "Edit" })).toHaveCSS(
      "text-decoration-line",
      "none",
    );
    await expect(popover.getByRole("link", { name: "History" })).toHaveCSS(
      "text-decoration-line",
      "none",
    );
    const triggerBox = await trigger.boundingBox();
    const popoverBox = await popover.boundingBox();

    expect(triggerBox).not.toBeNull();
    expect(popoverBox).not.toBeNull();
    expect(
      popoverBox!.y - (triggerBox!.y + triggerBox!.height),
    ).toBeGreaterThanOrEqual(0);
    expect(
      popoverBox!.y - (triggerBox!.y + triggerBox!.height),
    ).toBeLessThanOrEqual(8);
    const triggerRight = triggerBox!.x + triggerBox!.width;
    const popoverRight = popoverBox!.x + popoverBox!.width;
    const alignmentFitsViewport =
      triggerRight - popoverBox!.width >= 0 && triggerRight <= viewport.width;
    if (alignmentFitsViewport) {
      expect(
        Math.abs(popoverRight - triggerRight),
        JSON.stringify({ viewport, triggerBox, popoverBox }),
      ).toBeLessThanOrEqual(1);
    }
    expect(popoverBox!.x).toBeGreaterThanOrEqual(0);
    expect(popoverBox!.y).toBeGreaterThanOrEqual(0);
    expect(popoverBox!.x + popoverBox!.width).toBeLessThanOrEqual(
      viewport.width,
    );
    expect(popoverBox!.y + popoverBox!.height).toBeLessThanOrEqual(
      viewport.height,
    );

    await page.keyboard.press("Escape");
    await expect(popover).not.toBeVisible();
    await expect(trigger).toBeFocused();
  }

  const anonymousContext = await tracedContext();
  try {
    const anonymousPage = await anonymousContext.newPage();
    await goto(anonymousPage, post.permalink, { timeout: firstNav });
    await expect(
      anonymousPage.getByRole("button", { name: "Actions" }),
    ).toHaveCount(0);
    await expect(anonymousPage.locator(".j-trusted-post-actions")).toBeEmpty();
  } finally {
    await anonymousContext.close();
  }
});

test("owned Post Actions stay per-Post across supported routes", async ({
  page,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  const tag = `actions-${username}`;
  const firstPost = await createPostViaApi(page, {
    body: "# First route actions probe\n\nFirst",
    tags: [tag],
  });
  await createPostViaApi(page, {
    body: "# Second route actions probe\n\nSecond",
    tags: [tag],
  });
  await goto(page, `/~${username}`, { timeout: firstNav });

  const expectRouteActions = async (expectedPosts: number) => {
    const ownPosts = page.locator("article.j-post", {
      has: page.locator(".j-post-handle", {
        hasText: `@${username}`,
      }),
    });
    await expect(ownPosts).toHaveCount(expectedPosts);
    await expect(page.getByRole("button", { name: "Actions" })).toHaveCount(
      expectedPosts,
    );
    await expect(
      page
        .locator("#j-trusted-chrome")
        .getByRole("button", { name: "Actions" }),
    ).toHaveCount(0);
    await expect(page.getByText(/Actions for @/)).toHaveCount(0);
    for (let index = 0; index < expectedPosts; index += 1) {
      await expectPostActionsTextAlignment(page, ownPosts.nth(index));
    }
  };

  // The first route also waits for authenticated enhancement before any
  // in-app move, so later PostCards observe the settled SessionContext.
  await expectRouteActions(2);
  for (const destination of [
    { url: "/", ready: '.j-topbar h1:has-text("Jaunder")', expectedPosts: 2 },
    { url: "/app", ready: ".j-composer", expectedPosts: 2 },
    {
      url: `/~${username}`,
      ready: `.j-topbar h1:has-text("Posts by ${username}")`,
      expectedPosts: 2,
    },
    {
      url: `/tags/${tag}`,
      ready: `.j-topbar h1:has-text("#${tag}")`,
      expectedPosts: 2,
    },
    {
      url: firstPost.permalink,
      ready: `.j-topbar h1:has-text("Post by ${username}")`,
      expectedPosts: 1,
    },
  ]) {
    await navigateInApp(
      page,
      () =>
        page.evaluate((url) => {
          history.pushState({}, "", url);
          window.dispatchEvent(new PopStateEvent("popstate"));
        }, destination.url),
      destination,
    );
    await expectRouteActions(destination.expectedPosts);
  }
});

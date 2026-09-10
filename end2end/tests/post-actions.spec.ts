import { test, expect } from "./fixtures";
import { expectAccessible } from "./accessibility";
import { goto, signInAsNewUser } from "./helpers";
import { createPostViaApi, openPostActions } from "./posts";
import { navigateInApp } from "./navigate";

test("owner Post Actions disclosures use native popover dismissal and focus", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  await createPostViaApi(page, { body: "# First actions probe\n\nFirst" });
  await createPostViaApi(page, { body: "# Second actions probe\n\nSecond" });
  await goto(page, `/~${username}`, { timeout: firstNav });

  const triggers = page.getByRole("button", { name: "Actions" });
  await expect(triggers).toHaveCount(2);
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
  await expectAccessible(page);

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
  await triggers.nth(1).click();
  await expect(firstPopover).not.toBeVisible();
  await expect(page.locator(":popover-open")).toHaveCount(1);

  await page.setViewportSize({ width: 375, height: 800 });
  await page.locator("article.j-post").nth(1).scrollIntoViewIfNeeded();
  await expect(triggers.nth(1)).toBeInViewport();
  const narrowPopover = await page.locator(":popover-open").boundingBox();
  expect(narrowPopover).not.toBeNull();
  expect(narrowPopover!.x).toBeGreaterThanOrEqual(0);
  expect(narrowPopover!.x + narrowPopover!.width).toBeLessThanOrEqual(375);

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

test("owned Post Actions stay per-Post across supported routes", async ({
  page,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  const firstPost = await createPostViaApi(page, {
    body: "# First route actions probe\n\nFirst",
  });
  await createPostViaApi(page, {
    body: "# Second route actions probe\n\nSecond",
  });
  await goto(page, "/", { timeout: firstNav });

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
  };

  // The first route also waits for authenticated enhancement before any
  // in-app move, so later PostCards observe the settled SessionContext.
  await expectRouteActions(2);
  for (const destination of [
    { url: "/app", ready: ".j-composer", expectedPosts: 2 },
    {
      url: `/~${username}`,
      ready: `.j-topbar h1:has-text("Posts by ${username}")`,
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

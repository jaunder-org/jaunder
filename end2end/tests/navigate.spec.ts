/**
 * `navigateInApp` — in-app movement, and the barrier that keeps it honest
 * (#867).
 *
 * These use `registeredPage` because the `/app` nav item is authed-only
 * (`web/src/sidebar/markup.rs`, `auth_required = true`), so an anonymous page
 * has no link to click.
 *
 * The "no document load" assertion listens to `domcontentloaded` directly
 * rather than reading `bootBudget`: the budget is not armed automatically until
 * the fixture wiring lands, and a test that proves the mechanism should not
 * depend on the mechanism.
 */

import { expect } from "@playwright/test";
import { test } from "./fixtures";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";

/** The inline composer's body field — present on `/app`, absent on `/`. */
const APP_READY = SEL.postBody;
const APP_LINK = 'a[href="/app"]';

test("an in-app move changes route without a document load", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/");
  let loads = 0;
  page.on("domcontentloaded", () => {
    loads += 1;
  });

  await navigateInApp(page, () => page.click(APP_LINK), {
    url: "/app",
    ready: APP_READY,
  });

  expect(new URL(page.url()).pathname).toBe("/app");
  expect(loads).toBe(0);
});

test("Compose sidebar navigation reaches the full composer without a document load", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/");
  let loads = 0;
  page.on("domcontentloaded", () => {
    loads += 1;
  });

  const composeLink = page.locator('a[href="/posts/new"]');
  await expect(composeLink).toHaveText("Compose");
  await navigateInApp(page, () => composeLink.click(), {
    url: "/posts/new",
    ready: "#audience-base",
  });
  await expect(page.locator(SEL.topbarHeading)).toHaveText("New post");
  await expect(page.locator(".j-topbar")).toContainText("Long-form");
  expect(loads).toBe(0);
});

test("it fails loudly when the destination never renders", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/");
  await expect(
    navigateInApp(page, () => page.click(APP_LINK), {
      url: "/app",
      ready: "#never-rendered",
      timeoutMs: 2_000,
    }),
  ).rejects.toThrow();
});

test("it rejects a barrier that is already satisfied", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/");
  await expect(
    navigateInApp(page, () => page.click(APP_LINK), {
      url: "/app",
      ready: "body",
    }),
  ).rejects.toThrow(/already matches[\s\S]*waits for nothing/);
});

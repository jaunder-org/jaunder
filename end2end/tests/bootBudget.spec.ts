/**
 * Browser contracts for the per-page document-load budget (#867).
 *
 * Accounting transitions live in `bootBudgetAccounting.spec.ts`. These tests keep
 * only claims that require a real browser event, page lifecycle, or fixture.
 */

import { expect } from "@playwright/test";
import {
  allowSecondBoot,
  bootCount,
  takeBudgetFailures,
  trackBoots,
} from "./bootBudget";
import { test, testWithoutAutoBootBudget } from "./fixtures";
import { BASE_URL, goto } from "./helpers";

test("one real document load counts one boot", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  expect(bootCount(page)).toBe(1);
});

test("a same-document router push does not count", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  await page.evaluate(() => history.pushState({}, "", "/app"));
  await page.waitForFunction(() => location.pathname === "/app");
  expect(bootCount(page)).toBe(1);
});

test("a raw page.goto is counted by the page listener", async ({ page }) => {
  trackBoots(page);
  // e2e-goto-wrapper:allow records the entry load outside the wrapper
  await page.goto(`${BASE_URL}/`);
  // e2e-goto-wrapper:allow proves the listener sees loads the wrapper did not issue
  await page.goto(`${BASE_URL}/login`);

  expect(takeBudgetFailures()).toHaveLength(1);
});

testWithoutAutoBootBudget(
  "a declaration arms a page after its entry load",
  async ({ page }) => {
    // Automatic arming is disabled for this test: the declaration must observe
    // and record the page's already-completed entry load itself.
    await goto(page, "/");
    allowSecondBoot(page, "arming happens at declaration time here");
    await goto(page, "/login");
    expect(new URL(page.url()).pathname).toBe("/login");
  },
);

// `registeredPage` is the fixture form of the rule: it owns the first entry and
// rejects a second request rather than relying on callers to coordinate one.
test("registeredPage boots at the given entry", async ({ registeredPage }) => {
  const page = await registeredPage("/posts/new");
  expect(new URL(page.url()).pathname).toBe("/posts/new");
});

test("registeredPage refuses a second call", async ({ registeredPage }) => {
  await registeredPage("/posts/new");
  await expect(registeredPage("/profile")).rejects.toThrow(
    /called twice[\s\S]*\/posts\/new/,
  );
});

test("the fixture arms every test page", async ({ registeredPage }) => {
  const page = await registeredPage("/");
  expect(bootCount(page)).toBe(1);
});

test("a traced second page is armed and explicit arming is idempotent", async ({
  registeredPage,
  tracedContext,
}) => {
  await registeredPage("/");
  const context = await tracedContext();
  try {
    const other = await context.newPage();
    trackBoots(other);
    await goto(other, "/");
    expect(bootCount(other)).toBe(1);
  } finally {
    await context.close();
  }
});

test("an undeclared raw second load reaches the teardown sweep", async ({
  page,
}) => {
  await goto(page, "/");
  // e2e-goto-wrapper:allow no budget-aware helper runs after this load
  await page.goto(`${BASE_URL}/login`);

  const failures = takeBudgetFailures();
  expect(failures).toHaveLength(1);
  expect(failures[0]).toContain("undeclared second load");
  expect(failures[0]).toContain(`${BASE_URL}/login`);
  expect(takeBudgetFailures()).toEqual([]);
});

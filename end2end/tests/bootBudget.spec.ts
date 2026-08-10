/**
 * The per-page document-load budget (#867).
 *
 * These run in a real browser on purpose: the budget's central claim is about
 * which browser events fire for which kind of navigation, and no amount of unit
 * testing in node can check that. The second test is the load-bearing one — it
 * is what says a router push is not a boot.
 */

import { expect } from "@playwright/test";
import { allowSecondBoot, bootCount, trackBoots } from "./bootBudget";
import { test } from "./fixtures";
import { BASE_URL, goto } from "./helpers";

test("one document load counts one boot", async ({ page }) => {
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

test("a second document load is rejected when undeclared", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  // `[\s\S]` rather than `.` with the `s` flag: the suite's tsc target predates
  // es2018, where dotAll became available.
  await expect(goto(page, "/login")).rejects.toThrow(
    /second document load [\s\S]*\/login[\s\S]*allowSecondBoot/,
  );
});

test("a declared second document load is permitted", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  allowSecondBoot(page, "the login page's cold render is the subject");
  await goto(page, "/login");
  expect(bootCount(page)).toBe(2);
});

test("an allowance is consumed, not permanent", async ({ page }) => {
  trackBoots(page);
  await goto(page, "/");
  allowSecondBoot(page, "one extra load");
  await goto(page, "/login");
  await expect(goto(page, "/register")).rejects.toThrow(/second document load/);
});

test("a raw page.goto is counted too", async ({ page }) => {
  trackBoots(page);
  // The budget must not depend on the wrapper: sites that cannot use `goto`
  // (the CLS probe) are still pages, and still boot once.
  // e2e-goto-wrapper:allow proves the counter sees loads the wrapper never issued
  await page.goto(`${BASE_URL}/`);
  await expect(goto(page, "/login")).rejects.toThrow(/second document load/);
});

test("an empty reason is rejected", async ({ page }) => {
  trackBoots(page);
  expect(() => allowSecondBoot(page, "   ")).toThrow(/reason/);
});

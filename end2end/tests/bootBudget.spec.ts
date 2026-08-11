/**
 * The per-page document-load budget (#867).
 *
 * These run in a real browser on purpose: the budget's central claim is about
 * which browser events fire for which kind of navigation, and no amount of unit
 * testing in node can check that. The second test is the load-bearing one — it
 * is what says a router push is not a boot.
 */

import { expect } from "@playwright/test";
import {
  allowEngineDependentBoot,
  allowSecondBoot,
  bootCount,
  pendingReasons,
  takeBudgetFailures,
  trackBoots,
} from "./bootBudget";
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

test("a declaration works on a page armed late", async ({ page }) => {
  // No trackBoots: the declaration itself arms the page. A test may declare a
  // second load on a page the fixtures have not armed, and refusing that would
  // deadlock — declarations are written before arming becomes automatic.
  await goto(page, "/");
  allowSecondBoot(page, "arming happens at declaration time here");
  await goto(page, "/login");
  expect(new URL(page.url()).pathname).toBe("/login");
});

// `registeredPage` is the same one-boot rule expressed as a fixture: the test
// names its entry, and the fixture refuses to boot the page a second time.
// No `bootCount` assertion here — the fixture navigates before the test body
// can call `trackBoots`, so counting only works once arming is automatic.
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

// ── Automatic arming (Task 8) ────────────────────────────────────────────────

test("the budget is armed for every test's page", async ({
  registeredPage,
}) => {
  // No explicit `trackBoots` call — the `_autoPerfSpan` auto fixture must have
  // armed the page before `registeredPage` navigated it.
  const page = await registeredPage("/");
  expect(bootCount(page)).toBe(1);
});

test("the budget is armed for a second page too", async ({
  registeredPage,
  tracedContext,
}) => {
  await registeredPage("/");
  const context = await tracedContext();
  try {
    const other = await context.newPage();
    trackBoots(other); // must be idempotent — tracedContext already armed it
    await goto(other, "/");
    expect(bootCount(other)).toBe(1);
  } finally {
    await context.close();
  }
});

// ── Orphaned allowances (Task 8) ─────────────────────────────────────────────

test("an allowance nothing consumes is reported as an orphan", async ({
  page,
}) => {
  await goto(page, "/");
  allowSecondBoot(page, "a second load that never happens");

  // Collected here rather than left for teardown on purpose: an orphan left in
  // place fails the test it sits in, which is exactly the behaviour under test —
  // so asserting it that way would require a test that fails to pass. What
  // teardown does with this list is interpolate it into its error message, so
  // pinning the list pins the message.
  const orphans = takeBudgetFailures();
  expect(orphans).toHaveLength(1);
  // The line names the reason AND the page, so a multi-page test says which one.
  expect(orphans[0]).toContain("a second load that never happens");
  expect(orphans[0]).toContain(`${BASE_URL}/`);
  // Taking them clears them, so one orphan is reported once.
  expect(pendingReasons(page)).toEqual([]);
  expect(takeBudgetFailures()).toEqual([]);
});

test("a consumed allowance is not an orphan", async ({ page }) => {
  await goto(page, "/");
  allowSecondBoot(page, "the login page's cold render is the subject");
  await goto(page, "/login");
  expect(takeBudgetFailures()).toEqual([]);
});

test("an undeclared second load is reported even when no later goto raises it", async ({
  page,
}) => {
  await goto(page, "/");
  // e2e-goto-wrapper:allow the subject is a load the wrapper never sees, so nothing calls throwIfViolated and only the teardown sweep can surface it
  await page.goto(`${BASE_URL}/login`);

  // Collected here rather than left for teardown for the same reason as the
  // orphan test above: teardown would fail this test with the very violation it
  // is asserting. Pinning the list pins the message teardown interpolates.
  const failures = takeBudgetFailures();
  expect(failures).toHaveLength(1);
  expect(failures[0]).toContain("undeclared second load");
  expect(failures[0]).toContain(`${BASE_URL}/login`);
  // Taking clears it, so teardown does not report the same breach twice.
  expect(takeBudgetFailures()).toEqual([]);
});

// ── Engine-dependent loads (#867, found on firefox) ──────────────────────────

test("an engine-dependent allowance nothing consumes is not an orphan", async ({
  page,
}) => {
  await goto(page, "/");
  allowEngineDependentBoot(
    page,
    "/register",
    "a load only some engines produce, and this one did not",
  );
  // The whole point of the form: unconsumed is not evidence that the test
  // over-declared, because whether the load happens is not the test's choice.
  expect(takeBudgetFailures()).toEqual([]);
});

test("an engine-dependent allowance covers a load of its own path", async ({
  page,
}) => {
  await goto(page, "/");
  allowEngineDependentBoot(page, "/login", "a load some engines produce");
  await goto(page, "/login");
  expect(bootCount(page)).toBe(2);
  expect(pendingReasons(page)).toEqual([]);
  expect(takeBudgetFailures()).toEqual([]);
});

test("an engine-dependent allowance is inert against a load it does not name", async ({
  page,
}) => {
  await goto(page, "/");
  // The load this declaration describes never happens — the engine that produces
  // it is not this one. A DIFFERENT load then arrives, undeclared.
  allowEngineDependentBoot(
    page,
    "/register",
    "the load this engine did not produce",
  );
  // Unscoped, this allowance would be handed to `/login` and the undeclared load
  // would pass silently — the one thing the budget exists to catch. Scoped, it
  // matches nothing here and `/login` is reported.
  // e2e-goto-wrapper:allow a raw load keeps the breach out of `goto`, so the assertion reads the recorded failure rather than a thrown error
  await page.goto(`${BASE_URL}/login`);

  const failures = takeBudgetFailures();
  expect(failures).toHaveLength(1);
  expect(failures[0]).toContain("undeclared second load");
  expect(failures[0]).toContain(`${BASE_URL}/login`);
});

test("a scoped allowance is spent on its own load, not the exact one", async ({
  page,
}) => {
  await goto(page, "/");
  // The pre-paint shape, on the engine that produces BOTH loads: an exact
  // declaration for the load that always lands, and a scoped one for the load
  // only some engines produce. The scoped load must spend the scoped
  // declaration — spending the exact one here would leave the other load with
  // nothing to consume.
  allowSecondBoot(page, "the load that always happens");
  allowEngineDependentBoot(page, "/register", "the engine-dependent load");
  await goto(page, "/register");
  await goto(page, "/login");

  expect(bootCount(page)).toBe(3);
  expect(pendingReasons(page)).toEqual([]);
  expect(takeBudgetFailures()).toEqual([]);
});

test("an engine-dependent allowance still needs a reason", async ({ page }) => {
  await goto(page, "/");
  expect(() => allowEngineDependentBoot(page, "/register", "   ")).toThrow(
    /reason/,
  );
});

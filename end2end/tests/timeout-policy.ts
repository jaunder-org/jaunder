/**
 * E2E timeout-policy infrastructure.
 *
 * Owns browser- and contention-scaled budgets, the default whole-test budget,
 * and its duration-manifest handoff. The larger scale always wins;
 * `fixtures.ts` explicitly composes the automatic policy fixtures into the
 * suite's test surface.
 */

import type { TestInfo } from "@playwright/test";
import { attachEffectiveTimeout } from "../durationBudgetManifest";

type Use<T> = (value: T) => Promise<void>;

// Per-test budgets scale up for two independent reasons, and a test can hit
// either: a slow browser engine or worker CPU contention. The larger factor
// wins because Firefox's browser scale already absorbs measured contention.
const slowBrowserTimeoutScale = 2.2;
const slowBrowserFirstNavigationScale = 2.6;

function workerContentionScale(testInfo: TestInfo): number {
  const resolved = testInfo.config.workers;
  const workers = Number.isFinite(resolved) && resolved > 0 ? resolved : 1;
  if (workers <= 1) return 1.0;
  if (workers === 2) return 1.5;
  if (workers === 3) return 2.0;
  return 2.5;
}

export function slowBrowserTimeoutMs(
  testInfo: TestInfo,
  chromiumBudgetMs: number,
): number {
  const browserScale =
    testInfo.project.name === "chromium" ? 1.0 : slowBrowserTimeoutScale;
  return Math.ceil(
    chromiumBudgetMs * Math.max(browserScale, workerContentionScale(testInfo)),
  );
}

export function slowBrowserFirstNavigationTimeoutMs(
  testInfo: TestInfo,
  chromiumBudgetMs: number,
): number {
  const browserScale =
    testInfo.project.name === "chromium"
      ? 1.0
      : slowBrowserFirstNavigationScale;
  return Math.ceil(
    chromiumBudgetMs * Math.max(browserScale, workerContentionScale(testInfo)),
  );
}

/** The ambient whole-test budget every test receives via the auto fixture. */
export const DEFAULT_TEST_BUDGET_MS = 30_000;

let currentTestInfo: () => TestInfo;

export function setTestInfoAccessor(accessor: () => TestInfo): void {
  currentTestInfo = accessor;
}

/** Raise the current test's whole-test budget to a scaled Chromium budget. */
export function setTestBudget(chromiumBudgetMs: number): void {
  const info = currentTestInfo();
  info.setTimeout(slowBrowserTimeoutMs(info, chromiumBudgetMs));
}

type AutoFixture<T> = [
  (args: {}, use: Use<T>, testInfo: TestInfo) => Promise<void>,
  { auto: true },
];

export const autoTestTimeoutFixture = [
  async ({}, use: Use<void>, testInfo: TestInfo) => {
    testInfo.setTimeout(slowBrowserTimeoutMs(testInfo, DEFAULT_TEST_BUDGET_MS));
    await use();
  },
  { auto: true },
] satisfies AutoFixture<void>;

export const autoDurationBudgetFixture = [
  async ({}, use: Use<void>, testInfo: TestInfo) => {
    try {
      await use();
    } finally {
      await attachEffectiveTimeout(testInfo);
    }
  },
  { auto: true },
] satisfies AutoFixture<void>;

export const firstNavFixture = async (
  {},
  use: Use<number>,
  testInfo: TestInfo,
): Promise<void> => {
  await use(slowBrowserFirstNavigationTimeoutMs(testInfo, 10_000));
};

import { expect } from "@playwright/test";
import {
  DurationBudgetManifestCollector,
  type DurationBudgetManifest,
} from "../durationBudgetManifest";
import { test } from "./fixtures";
import {
  DEFAULT_TEST_BUDGET_MS,
  setTestBudget,
  slowBrowserTimeoutMs,
} from "./timeout-policy";

test("duration budget fixture records the resolved ambient timeout", ({}, testInfo) => {
  expect(testInfo.timeout).toBe(
    slowBrowserTimeoutMs(testInfo, DEFAULT_TEST_BUDGET_MS),
  );
});

test("duration budget fixture records the final slow-test timeout", ({}, testInfo) => {
  const ambientTimeout = testInfo.timeout;
  test.slow();
  expect(testInfo.timeout).toBe(ambientTimeout * 3);
});

test("duration budget fixture records an explicit derived timeout", ({}, testInfo) => {
  const chromiumBudgetMs = DEFAULT_TEST_BUDGET_MS + 1_000;
  setTestBudget(chromiumBudgetMs);
  expect(testInfo.timeout).toBe(
    slowBrowserTimeoutMs(testInfo, chromiumBudgetMs),
  );
});

test("manifest collector retains dependency-project retries", () => {
  const collector = new DurationBudgetManifestCollector([
    {
      test_id: "ordinary",
      project_id: "chromium",
      project_name: "chromium",
      title: "ordinary test",
      file: "duration-budget-manifest.spec.ts",
      line: 10,
    },
    {
      test_id: "dependent",
      project_id: "chromium-admin",
      project_name: "chromium-admin",
      title: "dependent project test",
      file: "duration-budget-manifest.spec.ts",
      line: 20,
    },
  ]);
  collector.observeAttempt("ordinary", 0);
  collector.recordBudget({
    test_id: "ordinary",
    retry: 0,
    effective_timeout_ms: 30_000,
  });
  collector.observeAttempt("dependent", 0);
  collector.recordBudget({
    test_id: "dependent",
    retry: 0,
    effective_timeout_ms: 45_000,
  });
  collector.observeAttempt("dependent", 1);
  collector.recordBudget({
    test_id: "dependent",
    retry: 1,
    effective_timeout_ms: 45_000,
  });

  const expected: DurationBudgetManifest = {
    schema_version: 1,
    complete: true,
    tests: [
      {
        test_id: "ordinary",
        project_id: "chromium",
        project_name: "chromium",
        title: "ordinary test",
        file: "duration-budget-manifest.spec.ts",
        line: 10,
        attempts: [{ retry: 0, effective_timeout_ms: 30_000 }],
      },
      {
        test_id: "dependent",
        project_id: "chromium-admin",
        project_name: "chromium-admin",
        title: "dependent project test",
        file: "duration-budget-manifest.spec.ts",
        line: 20,
        attempts: [
          { retry: 0, effective_timeout_ms: 45_000 },
          { retry: 1, effective_timeout_ms: 45_000 },
        ],
      },
    ],
  };
  expect(collector.manifest()).toEqual(expected);
});

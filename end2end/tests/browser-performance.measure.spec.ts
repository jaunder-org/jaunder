import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { performance } from "node:perf_hooks";
import type { Locator, Page } from "@playwright/test";
import { expect, setTestBudget, test, type NewTracedContext } from "./fixtures";

import {
  PERFORMANCE_PAGE_SIZE,
  PERFORMANCE_SAMPLE_COUNT,
  atomicWriteJson,
  browserDiagnosticArtifacts,
  browserWorkloads,
  performanceEnabled,
  readPerformanceEnvironment,
  summarize,
  validateManifest,
  type BrowserWorkload,
  type DatasetManifest,
} from "./browser-performance";
import { applySeededSession, createSessionViaTool } from "./seed";
import { goto } from "./helpers";

const enabled = performanceEnabled();
const BROWSER_PERFORMANCE_BUDGET_MS = 45 * 60_000;
test.skip(
  !enabled,
  "browser performance measurements require the explicit JAUNDER_PERF_* contract",
);

type SampleEvidence = {
  duration_us: number;
  rows_returned: number;
  navigation_artifact: string;
  trace_artifact: string;
};

function rowLocator(page: Page, workload: BrowserWorkload): Locator {
  return workload.workload === "home" || workload.workload === "app"
    ? page.locator('[data-jaunder-part="post-list"] article')
    : page.locator('[data-test="history-row"]');
}

function paginationButton(page: Page, workload: BrowserWorkload): Locator {
  return workload.workload === "home" || workload.workload === "app"
    ? page.getByRole("button", { name: "Load more", exact: true })
    : page.locator('[data-test="history-load-more"]');
}

async function expectHistoryField(
  page: Page,
  label: "Post ID" | "Revision ID",
  value: number,
): Promise<void> {
  const field = page
    .locator(".j-field-row")
    .filter({ has: page.getByText(label, { exact: true }) });
  await expect(field).toHaveCount(1);
  await expect(field.getByText(String(value), { exact: true })).toHaveCount(1);
}

function expectedInitialRows(
  manifest: DatasetManifest,
  workload: BrowserWorkload,
): number {
  if (workload.workload === "browser_revision_detail") return 1;
  const rows = manifest.subjects.browser_initial_rows;
  const expected =
    workload.workload === "home"
      ? rows.home
      : workload.workload === "app"
        ? rows.app
        : workload.workload === "global_history"
          ? rows.global_history
          : rows.post_history;
  return Math.min(expected, PERFORMANCE_PAGE_SIZE);
}

async function expectPaginationSettled(
  page: Page,
  workload: BrowserWorkload,
): Promise<void> {
  const button = paginationButton(page, workload);
  if ((await button.count()) !== 0) await expect(button).toBeEnabled();
}

async function waitForInitialReady(
  page: Page,
  manifest: DatasetManifest,
  workload: BrowserWorkload,
): Promise<number> {
  if (workload.workload === "home" || workload.workload === "app") {
    await expect(page.locator('[data-jaunder-part="post-list"]')).toBeVisible();
  } else if (workload.workload === "global_history") {
    await expect(
      page.locator('[data-test="history-page"] [data-test="history-list"]'),
    ).toBeVisible();
  } else if (workload.workload === "browser_post_history") {
    await expect(
      page.locator(
        '[data-test="post-history-page"] [data-test="history-current"]',
      ),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Post History", exact: true }),
    ).toBeVisible();
    await expectHistoryField(
      page,
      "Post ID",
      manifest.subjects.history_post_id,
    );
  } else {
    await expect(
      page.locator(
        '[data-test="history-detail-page"] [data-test="history-source"]',
      ),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Post Revision", exact: true }),
    ).toBeVisible();
    await expectHistoryField(
      page,
      "Post ID",
      manifest.subjects.history_post_id,
    );
    await expectHistoryField(
      page,
      "Revision ID",
      manifest.subjects.revision_id,
    );
    return 1;
  }
  const expected = expectedInitialRows(manifest, workload);
  await expect(rowLocator(page, workload)).toHaveCount(expected);
  await expectPaginationSettled(page, workload);
  return expected;
}

async function loadMoreAndWait(
  page: Page,
  workload: BrowserWorkload,
  previousRows: number,
  matchingRows: number,
): Promise<number> {
  const button = paginationButton(page, workload);
  await expect(button).toBeEnabled();
  await button.click();
  const expectedRows = Math.min(
    previousRows + PERFORMANCE_PAGE_SIZE,
    matchingRows,
  );
  await expect(rowLocator(page, workload)).toHaveCount(expectedRows);
  await expectPaginationSettled(page, workload);
  return expectedRows;
}

async function prepareDeepPosition(
  page: Page,
  manifest: DatasetManifest,
  workload: BrowserWorkload,
): Promise<number> {
  const cursor = workload.cursor;
  if (!cursor) throw new Error("deep workload lacks manifest cursor");
  let rows = await waitForInitialReady(page, manifest, workload);
  const setupActions = Math.max(
    0,
    Math.floor((cursor.resolved_rank - 1) / PERFORMANCE_PAGE_SIZE) - 1,
  );
  for (let action = 0; action < setupActions; action += 1) {
    rows = await loadMoreAndWait(
      page,
      workload,
      rows,
      cursor.matching_result_count,
    );
  }
  return rows;
}

async function measureSample(
  tracedContext: NewTracedContext,
  manifest: DatasetManifest,
  workload: BrowserWorkload,
  fragmentDirectory: string,
  sampleIndex: number,
): Promise<SampleEvidence> {
  const context = await tracedContext();
  const { navigation, trace } = browserDiagnosticArtifacts(
    fragmentDirectory,
    workload,
    sampleIndex,
  );
  const evidence: Array<{
    url: string;
    method: string;
    status: number | null;
  }> = [];
  let page: Page | undefined;
  try {
    await context.tracing.start({
      screenshots: true,
      snapshots: true,
      sources: false,
    });
    const session = await createSessionViaTool(
      manifest.subjects.username,
      `performance-${sampleIndex}`,
    );
    await applySeededSession(context, session);
    page = await context.newPage();
    page.on("response", (response) => {
      evidence.push({
        url: response.url(),
        method: response.request().method(),
        status: response.status(),
      });
    });

    if (workload.position === "deep") {
      await goto(page, workload.path);
      const rows = await prepareDeepPosition(page, manifest, workload);
      const started = performance.now();
      const observedRows = await loadMoreAndWait(
        page,
        workload,
        rows,
        workload.cursor!.matching_result_count,
      );
      return {
        duration_us: Math.round((performance.now() - started) * 1_000),
        rows_returned: observedRows - rows,
        navigation_artifact: navigation.reference,
        trace_artifact: trace.reference,
      };
    }

    const started = performance.now();
    await goto(page, workload.path);
    const rowsReturned = await waitForInitialReady(page, manifest, workload);
    return {
      duration_us: Math.round((performance.now() - started) * 1_000),
      rows_returned: rowsReturned,
      navigation_artifact: navigation.reference,
      trace_artifact: trace.reference,
    };
  } finally {
    try {
      await atomicWriteJson(navigation.destination, evidence);
    } finally {
      try {
        await context.tracing.stop({ path: trace.destination });
      } finally {
        await context.close();
      }
    }
  }
}

test("emits browser performance fragment from the authoritative manifest", async ({
  tracedContext,
}) => {
  setTestBudget(BROWSER_PERFORMANCE_BUDGET_MS);
  const environment = readPerformanceEnvironment();
  const manifest = validateManifest(
    JSON.parse(await readFile(environment.manifestPath, "utf8")),
  );
  const diagnostics = {
    navigation_artifacts: [] as string[],
    trace_artifacts: [] as string[],
    otel_trace_artifact: "diagnostics/otel-traces.jsonl",
  };
  const workloads = [];

  try {
    for (const workload of browserWorkloads(manifest)) {
      const samples: number[] = [];
      let rowsReturned = 0;
      for (
        let sampleIndex = 0;
        sampleIndex < PERFORMANCE_SAMPLE_COUNT;
        sampleIndex += 1
      ) {
        const artifacts = browserDiagnosticArtifacts(
          environment.fragmentDirectory,
          workload,
          sampleIndex,
        );
        diagnostics.navigation_artifacts.push(artifacts.navigation.reference);
        diagnostics.trace_artifacts.push(artifacts.trace.reference);
        const result = await measureSample(
          tracedContext,
          manifest,
          workload,
          environment.fragmentDirectory,
          sampleIndex,
        );
        samples.push(result.duration_us);
        if (result.rows_returned === 0) {
          throw new Error(
            `${workload.workload}:${workload.position} returned no observed rows`,
          );
        }
        if (sampleIndex === 0) {
          rowsReturned = result.rows_returned;
        } else if (rowsReturned !== result.rows_returned) {
          throw new Error(
            `${workload.workload}:${workload.position} rows_returned drifted from ${rowsReturned} to ${result.rows_returned}`,
          );
        }
      }
      workloads.push({
        key: {
          result_schema_version: 1,
          generator: manifest.plan.generator,
          profile: manifest.plan.profile,
          workload: workload.workload,
          backend: environment.backend,
          browser: environment.browser,
          build_mode: "release",
          measurement_frame: "cold",
          measurement_position: workload.position,
          sample_count: PERFORMANCE_SAMPLE_COUNT,
          page_size: workload.pageSize,
          // UI pagination exposes sequential pages rather than the persisted
          // cursor identity, so this browser action cannot claim that identity.
          cursor_target_percent: undefined,
          cursor_resolved_rank: undefined,
          ...environment.identity,
        },
        samples: samples.map((duration_us) => ({ duration_us })),
        summary: summarize(samples),
        rows_returned: rowsReturned,
      });
    }
  } catch (error) {
    await atomicWriteJson(
      join(
        environment.fragmentDirectory,
        "diagnostics",
        "browser-diagnostics.json",
      ),
      diagnostics,
    );
    throw error;
  }
  const expectedDiagnosticArtifacts =
    workloads.length * PERFORMANCE_SAMPLE_COUNT;
  if (
    diagnostics.navigation_artifacts.length !== expectedDiagnosticArtifacts ||
    diagnostics.trace_artifacts.length !== expectedDiagnosticArtifacts ||
    new Set(diagnostics.navigation_artifacts).size !==
      expectedDiagnosticArtifacts ||
    new Set(diagnostics.trace_artifacts).size !== expectedDiagnosticArtifacts
  ) {
    throw new Error(
      "browser diagnostics must contain one unique navigation and trace artifact per sample",
    );
  }
  if (
    diagnostics.navigation_artifacts.length === 0 ||
    diagnostics.trace_artifacts.length === 0
  )
    throw new Error("browser diagnostics must be retained");
  await atomicWriteJson(
    join(
      environment.fragmentDirectory,
      `browser-${environment.backend}-${environment.browser}-v1.json`,
    ),
    {
      schema_version: 1,
      manifest,
      fragment: {
        producer: "browser",
        result: {
          setup: {
            producer: "browser",
            backend: environment.backend,
            ...environment.setup,
          },
          diagnostics,
          workloads,
        },
      },
    },
  );
});

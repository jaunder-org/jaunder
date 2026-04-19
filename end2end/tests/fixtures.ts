import { expect, test as base, type Request } from "@playwright/test";
import { drainActionsForTest, setCurrentActionTestKey } from "./actions";
import {
  buildSpan,
  exportSpans,
  makeEvent,
  otlpAttribute,
  traceContextFromEnvironment,
} from "./otel";

type RequestRecord = {
  method: string;
  url: string;
  resourceType: string;
  startedMs: number;
  endedMs: number;
  durationMs: number;
  failed: boolean;
  failureText?: string;
};

type PagePerfSummary = {
  navigation: {
    domContentLoadedMs: number;
    loadMs: number;
    responseStartMs: number;
  } | null;
  resources: {
    count: number;
    totalDurationMs: number;
    topSlow: Array<{ name: string; initiatorType: string; durationMs: number }>;
  };
  longTasks: Array<{ startTime: number; duration: number; name: string }>;
};

const test = base.extend<{ _autoPerfSpan: void }>({
  _autoPerfSpan: [
    async ({ page }, use, testInfo) => {
      const traceContext = traceContextFromEnvironment();
      const testStartMs = Date.now();
      const testKey = `${testInfo.file}::${testInfo.title}::${testInfo.project.name}::${testInfo.retry}`;
      const requestStarts = new Map<Request, number>();
      const requests: RequestRecord[] = [];

      await page.addInitScript(() => {
        const globalScope = globalThis as typeof globalThis & {
          __jaunderLongTasks?: Array<{
            startTime: number;
            duration: number;
            name: string;
          }>;
        };
        globalScope.__jaunderLongTasks = [];

        if (typeof PerformanceObserver === "undefined") return;
        try {
          const observer = new PerformanceObserver((list) => {
            for (const entry of list.getEntries()) {
              if (entry.entryType !== "longtask") continue;
              globalScope.__jaunderLongTasks?.push({
                startTime: entry.startTime,
                duration: entry.duration,
                name: entry.name || "longtask",
              });
            }
          });
          observer.observe({ type: "longtask", buffered: true });
        } catch {
          // LongTask API is not available in every engine build.
        }
      });

      page.on("request", (request) => {
        requestStarts.set(request, Date.now());
      });

      page.on("requestfinished", (request) => {
        const startedMs = requestStarts.get(request) ?? Date.now();
        const endedMs = Date.now();
        requests.push({
          method: request.method(),
          url: request.url(),
          resourceType: request.resourceType(),
          startedMs,
          endedMs,
          durationMs: endedMs - startedMs,
          failed: false,
        });
      });

      page.on("requestfailed", (request) => {
        const startedMs = requestStarts.get(request) ?? Date.now();
        const endedMs = Date.now();
        requests.push({
          method: request.method(),
          url: request.url(),
          resourceType: request.resourceType(),
          startedMs,
          endedMs,
          durationMs: endedMs - startedMs,
          failed: true,
          failureText: request.failure()?.errorText,
        });
      });

      setCurrentActionTestKey(testKey);
      try {
        await use();
      } finally {
        setCurrentActionTestKey(null);
      }

      const endMs = Date.now();
      const actions = drainActionsForTest(testKey);
      let pagePerfSummary: PagePerfSummary = {
        navigation: null,
        resources: { count: 0, totalDurationMs: 0, topSlow: [] },
        longTasks: [],
      };

      try {
        pagePerfSummary = await page.evaluate(() => {
          const navigation = performance.getEntriesByType("navigation")[0] as
            | PerformanceNavigationTiming
            | undefined;
          const resources = performance.getEntriesByType(
            "resource",
          ) as PerformanceResourceTiming[];
          const longTasks = (
            (
              globalThis as typeof globalThis & {
                __jaunderLongTasks?: Array<{
                  startTime: number;
                  duration: number;
                  name: string;
                }>;
              }
            ).__jaunderLongTasks ?? []
          ).slice(-20);

          const topSlow = resources
            .map((resource) => ({
              name: resource.name,
              initiatorType: resource.initiatorType,
              durationMs: resource.duration,
            }))
            .sort((left, right) => right.durationMs - left.durationMs)
            .slice(0, 20);

          const totalDurationMs = resources.reduce(
            (sum, resource) => sum + resource.duration,
            0,
          );

          return {
            navigation: navigation
              ? {
                  domContentLoadedMs:
                    navigation.domContentLoadedEventEnd - navigation.startTime,
                  loadMs: navigation.loadEventEnd - navigation.startTime,
                  responseStartMs:
                    navigation.responseStart - navigation.startTime,
                }
              : null,
            resources: {
              count: resources.length,
              totalDurationMs,
              topSlow,
            },
            longTasks,
          };
        });
      } catch {
        // Page may already be closed on failure paths.
      }

      const sortedRequests = [...requests].sort(
        (left, right) => right.durationMs - left.durationMs,
      );
      const slowRequests = sortedRequests.filter(
        (request) => request.durationMs >= 500,
      );
      const topSlowRequests = sortedRequests.slice(0, 20);
      const topActions = [...actions]
        .sort((left, right) => right.durationMs - left.durationMs)
        .slice(0, 30);

      const attributes = [
        otlpAttribute("e2e.file", testInfo.file),
        otlpAttribute("e2e.test", testInfo.title),
        otlpAttribute("e2e.project", testInfo.project.name),
        otlpAttribute("e2e.status", testInfo.status),
        otlpAttribute("e2e.expected_status", testInfo.expectedStatus),
        otlpAttribute("e2e.retry", testInfo.retry),
        otlpAttribute("e2e.timeout_ms", testInfo.timeout),
        otlpAttribute("e2e.total_ms", endMs - testStartMs),
        otlpAttribute("e2e.request_count", requests.length),
        otlpAttribute(
          "e2e.request_failed_count",
          requests.filter((request) => request.failed).length,
        ),
        otlpAttribute("e2e.request_slow_count", slowRequests.length),
        otlpAttribute(
          "e2e.request_top_slow_json",
          JSON.stringify(topSlowRequests),
        ),
        otlpAttribute(
          "e2e.navigation_json",
          JSON.stringify(pagePerfSummary.navigation),
        ),
        otlpAttribute(
          "e2e.resource_summary_json",
          JSON.stringify(pagePerfSummary.resources),
        ),
        otlpAttribute(
          "e2e.long_tasks_json",
          JSON.stringify(pagePerfSummary.longTasks),
        ),
        otlpAttribute("e2e.action_count", actions.length),
        otlpAttribute("e2e.action_top_json", JSON.stringify(topActions)),
      ].filter(
        (attribute): attribute is NonNullable<typeof attribute> =>
          attribute !== null,
      );

      const requestEvents = topSlowRequests.map((request) =>
        makeEvent(
          request.failed ? "request.failed" : "request.slow",
          request.endedMs,
          [
            otlpAttribute("http.method", request.method),
            otlpAttribute("url.full", request.url),
            otlpAttribute("browser.resource_type", request.resourceType),
            otlpAttribute("duration_ms", request.durationMs),
            otlpAttribute("request.failed", request.failed),
            otlpAttribute("request.failure_text", request.failureText ?? null),
          ].filter(
            (attribute): attribute is NonNullable<typeof attribute> =>
              attribute !== null,
          ),
        ),
      );
      const actionEvents = topActions.map((action) =>
        makeEvent(
          action.ok ? "action.timed" : "action.failed",
          action.endedMs,
          [
            otlpAttribute("action.name", action.name),
            otlpAttribute("duration_ms", action.durationMs),
            otlpAttribute("action.ok", action.ok),
            otlpAttribute("page.url", action.pageUrl ?? null),
            otlpAttribute("action.error", action.error ?? null),
          ].filter(
            (attribute): attribute is NonNullable<typeof attribute> =>
              attribute !== null,
          ),
        ),
      );

      const span = buildSpan({
        traceContext,
        name: "e2e.test",
        kind: "client",
        startMs: testStartMs,
        endMs,
        attributes,
        events: [...requestEvents, ...actionEvents],
      });

      try {
        await exportSpans([span]);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        console.warn(`[e2e-otel] test export failed: ${message}`);
      }
    },
    { auto: true },
  ],
});

export { expect, test };

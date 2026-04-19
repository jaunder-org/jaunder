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
  hydrationRuntime: {
    hydrationMs: number | null;
    navigationMs: number | null;
    wasmTransferBytes: number | null;
    wasmResourceMs: number | null;
    wasmInitMs: number | null;
    leptosHydrateMs: number | null;
    postHydrateEffectsMs: number | null;
  } | null;
};

type HydrationPerfPayload = {
  hydration_ms?: unknown;
  navigation_ms?: unknown;
  wasm_transfer_bytes?: unknown;
  wasm_resource_ms?: unknown;
  wasm_init_ms?: unknown;
  leptos_hydrate_ms?: unknown;
  post_hydrate_effects_ms?: unknown;
};

type NavigationRecord = {
  id: number;
  url: string;
  startedMs: number;
  committedMs: number | null;
  domContentLoadedMs: number | null;
  loadMs: number | null;
  hydratedMs: number | null;
  requestFinishedMs: number | null;
  requestFailed: boolean;
  requestFailureText?: string;
  wasmInitMs: number | null;
  leptosHydrateMs: number | null;
  postHydrateEffectsMs: number | null;
};

type NavigationSummary = {
  id: number;
  url: string;
  totalMs: number;
  requestMs: number | null;
  commitToDomContentLoadedMs: number | null;
  commitToHydrationMs: number | null;
  domContentLoadedToLoadMs: number | null;
  loadToHydrationMs: number | null;
  requestFailed: boolean;
  wasmInitMs: number | null;
  leptosHydrateMs: number | null;
  postHydrateEffectsMs: number | null;
};

const test = base.extend<{ _autoPerfSpan: void }>({
  _autoPerfSpan: [
    async ({ page }, use, testInfo) => {
      const traceContext = traceContextFromEnvironment();
      const testStartMs = Date.now();
      const testKey = `${testInfo.file}::${testInfo.title}::${testInfo.project.name}::${testInfo.retry}`;
      const requestStarts = new Map<Request, number>();
      const requests: RequestRecord[] = [];
      const navigationRequestIds = new Map<Request, number>();
      const pendingNavigationIds: number[] = [];
      const navigations: NavigationRecord[] = [];
      let activeNavigationId: number | null = null;
      let nextNavigationId = 1;

      await page.exposeBinding("__jaunderRecordHydration", (_source, value) => {
        if (!value || typeof value !== "object") return;
        const payload = value as {
          href?: unknown;
          perf?: HydrationPerfPayload;
        };
        const href = typeof payload.href === "string" ? payload.href : null;
        const nowMs = Date.now();
        const perf = payload.perf;
        const parseMetric = (metric: unknown): number | null =>
          typeof metric === "number" && Number.isFinite(metric) ? metric : null;

        // Hydration should be attributed to the most recent matching navigation.
        for (let index = navigations.length - 1; index >= 0; index -= 1) {
          const navigation = navigations[index];
          if (navigation.hydratedMs !== null) continue;
          if (href !== null && navigation.url !== href) continue;
          navigation.hydratedMs = nowMs;
          if (perf && typeof perf === "object") {
            navigation.wasmInitMs = parseMetric(perf.wasm_init_ms);
            navigation.leptosHydrateMs = parseMetric(perf.leptos_hydrate_ms);
            navigation.postHydrateEffectsMs = parseMetric(
              perf.post_hydrate_effects_ms,
            );
          }
          return;
        }
      });

      await page.addInitScript(() => {
        const globalScope = globalThis as typeof globalThis & {
          __jaunderLongTasks?: Array<{
            startTime: number;
            duration: number;
            name: string;
          }>;
          __jaunder_perf?: unknown;
          __jaunderHydrationNotified?: boolean;
          __jaunderRecordHydration?: (payload: {
            href: string;
            perf?: unknown;
          }) => void;
        };
        globalScope.__jaunderLongTasks = [];
        globalScope.__jaunderHydrationNotified = false;

        const notifyHydration = () => {
          if (globalScope.__jaunderHydrationNotified) return;
          const body = document.body;
          if (!body || !body.hasAttribute("data-hydrated")) return;
          globalScope.__jaunderHydrationNotified = true;
          try {
            globalScope.__jaunderRecordHydration?.({
              href: location.href,
              perf: globalScope.__jaunder_perf,
            });
          } catch {
            // Ignore cross-context bridge errors while collecting diagnostics.
          }
        };

        notifyHydration();
        if (document.readyState === "loading") {
          document.addEventListener("DOMContentLoaded", notifyHydration, {
            once: true,
          });
        }
        try {
          const hydrationObserver = new MutationObserver(() =>
            notifyHydration(),
          );
          hydrationObserver.observe(document.documentElement, {
            subtree: true,
            attributes: true,
            attributeFilter: ["data-hydrated"],
          });
        } catch {
          // MutationObserver should always exist in browsers, but keep this defensive.
        }

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
        if (
          request.isNavigationRequest() &&
          request.resourceType() === "document" &&
          request.frame() === page.mainFrame()
        ) {
          const navigationId = nextNavigationId;
          nextNavigationId += 1;
          navigationRequestIds.set(request, navigationId);
          pendingNavigationIds.push(navigationId);
          navigations.push({
            id: navigationId,
            url: request.url(),
            startedMs: Date.now(),
            committedMs: null,
            domContentLoadedMs: null,
            loadMs: null,
            hydratedMs: null,
            requestFinishedMs: null,
            requestFailed: false,
            wasmInitMs: null,
            leptosHydrateMs: null,
            postHydrateEffectsMs: null,
          });
        }
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

        const navigationId = navigationRequestIds.get(request);
        if (navigationId !== undefined) {
          const navigation = navigations.find(
            (entry) => entry.id === navigationId,
          );
          if (navigation) {
            navigation.requestFinishedMs = endedMs;
            navigation.url = request.url();
          }
        }
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

        const navigationId = navigationRequestIds.get(request);
        if (navigationId !== undefined) {
          const navigation = navigations.find(
            (entry) => entry.id === navigationId,
          );
          if (navigation) {
            navigation.requestFinishedMs = endedMs;
            navigation.requestFailed = true;
            navigation.requestFailureText = request.failure()?.errorText;
            navigation.url = request.url();
          }
        }
      });

      page.on("framenavigated", (frame) => {
        if (frame !== page.mainFrame()) return;
        const navigationId = pendingNavigationIds.shift() ?? null;
        if (navigationId === null) return;
        activeNavigationId = navigationId;
        const navigation = navigations.find(
          (entry) => entry.id === navigationId,
        );
        if (!navigation) return;
        navigation.committedMs = Date.now();
        navigation.url = frame.url();
      });

      page.on("domcontentloaded", () => {
        if (activeNavigationId === null) return;
        const navigation = navigations.find(
          (entry) => entry.id === activeNavigationId,
        );
        if (!navigation) return;
        if (navigation.domContentLoadedMs === null) {
          navigation.domContentLoadedMs = Date.now();
        }
      });

      page.on("load", () => {
        if (activeNavigationId === null) return;
        const navigation = navigations.find(
          (entry) => entry.id === activeNavigationId,
        );
        if (!navigation) return;
        if (navigation.loadMs === null) {
          navigation.loadMs = Date.now();
        }
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
        hydrationRuntime: null,
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
                __jaunder_perf?: {
                  hydration_ms?: number;
                  navigation_ms?: number;
                  wasm_transfer_bytes?: number;
                  wasm_resource_ms?: number;
                  wasm_init_ms?: number;
                  leptos_hydrate_ms?: number;
                  post_hydrate_effects_ms?: number;
                };
              }
            ).__jaunderLongTasks ?? []
          ).slice(-20);
          const hydrationRuntime =
            (
              globalThis as typeof globalThis & {
                __jaunder_perf?: {
                  hydration_ms?: number;
                  navigation_ms?: number;
                  wasm_transfer_bytes?: number;
                  wasm_resource_ms?: number;
                  wasm_init_ms?: number;
                  leptos_hydrate_ms?: number;
                  post_hydrate_effects_ms?: number;
                };
              }
            ).__jaunder_perf ?? null;

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
            hydrationRuntime: hydrationRuntime
              ? {
                  hydrationMs: hydrationRuntime.hydration_ms ?? null,
                  navigationMs: hydrationRuntime.navigation_ms ?? null,
                  wasmTransferBytes:
                    hydrationRuntime.wasm_transfer_bytes ?? null,
                  wasmResourceMs: hydrationRuntime.wasm_resource_ms ?? null,
                  wasmInitMs: hydrationRuntime.wasm_init_ms ?? null,
                  leptosHydrateMs: hydrationRuntime.leptos_hydrate_ms ?? null,
                  postHydrateEffectsMs:
                    hydrationRuntime.post_hydrate_effects_ms ?? null,
                }
              : null,
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
      const navigationSummary: NavigationSummary[] = navigations
        .map((navigation) => {
          const endMs =
            navigation.hydratedMs ??
            navigation.loadMs ??
            navigation.domContentLoadedMs ??
            navigation.requestFinishedMs ??
            navigation.committedMs ??
            navigation.startedMs;
          const requestMs =
            navigation.committedMs !== null
              ? navigation.committedMs - navigation.startedMs
              : null;
          const commitToDomContentLoadedMs =
            navigation.committedMs !== null &&
            navigation.domContentLoadedMs !== null
              ? navigation.domContentLoadedMs - navigation.committedMs
              : null;
          const commitToHydrationMs =
            navigation.committedMs !== null && navigation.hydratedMs !== null
              ? navigation.hydratedMs - navigation.committedMs
              : null;
          const domContentLoadedToLoadMs =
            navigation.domContentLoadedMs !== null && navigation.loadMs !== null
              ? navigation.loadMs - navigation.domContentLoadedMs
              : null;
          const loadToHydrationMs =
            navigation.loadMs !== null && navigation.hydratedMs !== null
              ? navigation.hydratedMs - navigation.loadMs
              : null;
          return {
            id: navigation.id,
            url: navigation.url,
            totalMs: endMs - navigation.startedMs,
            requestMs,
            commitToDomContentLoadedMs,
            commitToHydrationMs,
            domContentLoadedToLoadMs,
            loadToHydrationMs,
            requestFailed: navigation.requestFailed,
            wasmInitMs: navigation.wasmInitMs,
            leptosHydrateMs: navigation.leptosHydrateMs,
            postHydrateEffectsMs: navigation.postHydrateEffectsMs,
          };
        })
        .sort((left, right) => right.totalMs - left.totalMs);
      const topNavigations = navigationSummary.slice(0, 20);

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
        otlpAttribute(
          "e2e.hydration_runtime_json",
          JSON.stringify(pagePerfSummary.hydrationRuntime),
        ),
        otlpAttribute("e2e.action_count", actions.length),
        otlpAttribute("e2e.action_top_json", JSON.stringify(topActions)),
        otlpAttribute("e2e.navigation_count", navigations.length),
        otlpAttribute(
          "e2e.navigation_top_json",
          JSON.stringify(topNavigations),
        ),
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
      const navigationEvents = topNavigations.map((navigation) =>
        makeEvent("navigation.lifecycle", endMs, [
          otlpAttribute("navigation.id", navigation.id),
          otlpAttribute("url.full", navigation.url),
          otlpAttribute("duration_ms", navigation.totalMs),
          otlpAttribute("navigation.request_ms", navigation.requestMs),
          otlpAttribute(
            "navigation.commit_to_domcontentloaded_ms",
            navigation.commitToDomContentLoadedMs,
          ),
          otlpAttribute(
            "navigation.commit_to_hydration_ms",
            navigation.commitToHydrationMs,
          ),
          otlpAttribute(
            "navigation.domcontentloaded_to_load_ms",
            navigation.domContentLoadedToLoadMs,
          ),
          otlpAttribute(
            "navigation.load_to_hydration_ms",
            navigation.loadToHydrationMs,
          ),
          otlpAttribute("navigation.wasm_init_ms", navigation.wasmInitMs),
          otlpAttribute(
            "navigation.leptos_hydrate_ms",
            navigation.leptosHydrateMs,
          ),
          otlpAttribute(
            "navigation.post_hydrate_effects_ms",
            navigation.postHydrateEffectsMs,
          ),
          otlpAttribute("navigation.request_failed", navigation.requestFailed),
        ]),
      );

      const span = buildSpan({
        traceContext,
        name: "e2e.test",
        kind: "client",
        startMs: testStartMs,
        endMs,
        attributes,
        events: [...requestEvents, ...actionEvents, ...navigationEvents],
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

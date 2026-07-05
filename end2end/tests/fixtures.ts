/**
 * Auto-applied Playwright fixture (`_autoPerfSpan`, `auto: true`) that wraps every
 * test in OTel capture: it instruments page requests, navigations, and hydration,
 * folds in the action records from actions.ts, and emits a single `e2e.test` span
 * on teardown. Also exports the slow-browser / worker-contention timeout scalers
 * tests use to size their per-browser budgets.
 */

import {
  expect,
  test as base,
  type Page,
  type Request,
  type TestInfo,
} from "@playwright/test";
import { drainActionsForTest, setCurrentActionTestKey } from "./actions";
import {
  buildSpan,
  exportSpans,
  makeEvent,
  otlpAttribute,
  traceContextFromEnvironment,
} from "./otel";
import { goto, login, register } from "./helpers";
import { readEmailLines, type CapturedEmail } from "./mail";

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
};

type NavigationSummary = {
  id: number;
  url: string;
  cacheWarmth: "cold" | "warm";
  totalMs: number;
  requestMs: number | null;
  commitToDomContentLoadedMs: number | null;
  commitToMountMs: number | null;
  domContentLoadedToLoadMs: number | null;
  requestFailed: boolean;
};

// Per-test budgets scale up for two independent reasons, and a test can hit
// either:
//   1. Slow browser engine — Firefox/WebKit execute the Leptos WASM bundle far
//      slower than Chromium (measured ~1.8x per-test on CSR, #155). The first
//      (cold-cache) navigation also pays the full WASM download + init, so it
//      uses a larger multiplier than steady-state.
//   2. Worker CPU contention — running >1 Playwright worker oversubscribes the
//      VM CPU (the CI runner is ~4 vCPU), slowing every test's client render.
// The budget takes the LARGER of the two factors, not the product: Firefox's
// browser scale already absorbs 4-worker contention empirically (66/66 green at
// workers=4, #155 AC3), while Chromium — which has no browser scale — would
// otherwise have zero headroom and its heavy tests time out under parallelism.
const slowBrowserTimeoutScale = 2.2;
const slowBrowserFirstNavigationScale = 2.6;

// CPU-contention headroom as a function of the Playwright worker count.
// Calibrated so 4 workers reaches Firefox's proven 2.2x; intermediate counts
// interpolate. 1 worker = no contention.
//
// The count comes from `testInfo.config.workers` — the value Playwright actually
// resolved from the config's `workers` setting — NOT a second read of
// JAUNDER_E2E_WORKERS. The env is read in exactly one place (the config's
// `workers`); everything downstream derives from Playwright's resolved value, so
// the budget scale can never disagree with the number of workers actually
// running. (Reading the env here with its own default silently diverged from the
// config default and applied zero headroom while N>1 workers ran — #155.)
function workerContentionScale(testInfo: TestInfo): number {
  const resolved = testInfo.config.workers;
  const workers = Number.isFinite(resolved) && resolved > 0 ? resolved : 1;
  if (workers <= 1) return 1.0;
  if (workers === 2) return 1.5;
  if (workers === 3) return 2.0;
  return 2.5; // 4+ workers: heaviest oversubscription on a ~4-vCPU runner
}
const defaultWarmupUrl = "http://localhost:3000/";
const defaultWarmupTimeoutMs = 10_000;

function parseBooleanFlag(raw: string | undefined): boolean {
  if (raw === undefined) {
    return false;
  }
  const normalized = raw.trim().toLowerCase();
  return (
    normalized === "1" ||
    normalized === "true" ||
    normalized === "yes" ||
    normalized === "on"
  );
}

function parseWarmupTimeoutMs(raw: string | undefined): number {
  if (raw === undefined) {
    return defaultWarmupTimeoutMs;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return defaultWarmupTimeoutMs;
  }
  return parsed;
}

async function warmupPageContext(
  page: Page,
  testInfo: TestInfo,
): Promise<void> {
  if (!parseBooleanFlag(process.env.JAUNDER_E2E_WARMUP)) {
    return;
  }

  const warmupUrl = process.env.JAUNDER_E2E_WARMUP_URL ?? defaultWarmupUrl;
  const timeoutMs = parseWarmupTimeoutMs(
    process.env.JAUNDER_E2E_WARMUP_TIMEOUT_MS,
  );

  try {
    await page.goto(warmupUrl, {
      waitUntil: "domcontentloaded",
      timeout: timeoutMs,
    });
    await page.waitForSelector("body[data-hydrated]", {
      timeout: timeoutMs,
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.warn(
      `[e2e-warmup] ${testInfo.project.name}: warmup failed for ${warmupUrl}: ${message}`,
    );
  }
}

export async function maybeWarmupPage(
  page: Page,
  testInfo: TestInfo,
): Promise<void> {
  await warmupPageContext(page, testInfo);
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

/** A uniquely-named account provisioned per test. `password` is the literal
 *  `register()` password; `email` is the deterministic unique address this
 *  account uses when it sets/verifies email. */
export type TestUser = { username: string; password: string; email: string };

/** A recipient-scoped mail waiter bound to one `TestUser.email`. Each call
 *  returns that recipient's next unseen message (FIFO), so parallel tests
 *  never consume each other's mail. */
export type Mailbox = {
  waitForNewEmail(timeoutMs?: number): Promise<CapturedEmail>;
};

const test = base.extend<{
  _autoPerfSpan: void;
  user: TestUser;
  mailbox: Mailbox;
  verifiedUser: TestUser;
}>({
  // A uniquely-named account, registered in a throwaway context so the test's
  // own `page` stays logged out. Lazy: only provisioned for tests that
  // destructure `user`.
  user: async ({ browser }, use, testInfo) => {
    const context = await browser.newContext();
    const page = await context.newPage();
    const username = await register(
      page,
      slowBrowserFirstNavigationTimeoutMs(testInfo, 15_000),
    );
    await context.close();
    await use({
      username,
      password: "testpassword123",
      email: `${username}@example.com`,
    });
  },

  // Recipient-scoped mail waiter. Filters mail.jsonl by `user.email` and tracks
  // a per-mailbox cursor so each call returns this recipient's next message.
  mailbox: async ({ user }, use) => {
    const matching = () =>
      readEmailLines()
        .map((line) => JSON.parse(line) as CapturedEmail)
        .filter((mail) => mail.to.includes(user.email));
    // Seed the cursor at any pre-existing matching mail (there should be none,
    // since the address is unique to this test).
    let cursor = matching().length;
    const waitForNewEmail = async (
      timeoutMs = 5_000,
    ): Promise<CapturedEmail> => {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        const mails = matching();
        if (mails.length > cursor) {
          const next = mails[cursor];
          cursor += 1;
          return next;
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      throw new Error(`timed out waiting for email to ${user.email}`);
    };
    await use({ waitForNewEmail });
  },

  // `user` plus the email set-and-verify flow, driven through `mailbox`, all
  // out-of-band so the test's `page` stays logged out. Yields the same
  // credentials; the account now has a verified email.
  verifiedUser: async ({ browser, user, mailbox }, use, testInfo) => {
    // Fixture setup (newContext + login + set-email + verify) runs BEFORE the
    // test body, so a `test.setTimeout(...)` in the body is too late to cover
    // it — this expensive out-of-band flow would run under the un-scaled 30s
    // default and time out under worker CPU contention (#155, workers=4). Scale
    // the whole test's budget here, at the fixture's start, so setup is covered.
    testInfo.setTimeout(slowBrowserTimeoutMs(testInfo, 30_000));
    const context = await browser.newContext();
    const page = await context.newPage();
    const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 15_000);
    await login(page, user.username, user.password, firstNav);
    await goto(page, "/profile/email");
    await page.fill('input[name="email"]', user.email);
    await page.click('button[type="submit"]');
    await expect(page.locator('p:has-text("Check your email")')).toBeVisible({
      timeout: 10_000,
    });
    const mail = await mailbox.waitForNewEmail();
    const tokenMatch = mail.body_text.match(/token=([^\s]+)/);
    if (!tokenMatch) throw new Error("no verification token in captured email");
    await goto(page, `/verify-email?token=${tokenMatch[1]}`);
    await expect(page.locator('p:has-text("verified")')).toBeVisible();
    await context.close();
    await use(user);
  },

  _autoPerfSpan: [
    async ({ page }, use, testInfo) => {
      // Optional experiment mode: warm the same browser context before tracing starts.
      await warmupPageContext(page, testInfo);

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
        const payload = value as { href?: unknown };
        const href = typeof payload.href === "string" ? payload.href : null;
        const nowMs = Date.now();

        // The mount-ready marker should be attributed to the most recent matching
        // navigation (CSR has no hydration; `data-hydrated` marks mount-ready).
        for (let index = navigations.length - 1; index >= 0; index -= 1) {
          const navigation = navigations[index];
          if (navigation.hydratedMs !== null) continue;
          if (href !== null && navigation.url !== href) continue;
          navigation.hydratedMs = nowMs;
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
          __jaunderHydrationNotified?: boolean;
          __jaunderRecordHydration?: (payload: { href: string }) => void;
        };
        globalScope.__jaunderLongTasks = [];
        globalScope.__jaunderHydrationNotified = false;

        const notifyHydration = () => {
          if (globalScope.__jaunderHydrationNotified) return;
          const body = document.body;
          if (!body || !body.hasAttribute("data-hydrated")) return;
          globalScope.__jaunderHydrationNotified = true;
          try {
            globalScope.__jaunderRecordHydration?.({ href: location.href });
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
      const navigationSummary: NavigationSummary[] = navigations
        .map((navigation): NavigationSummary => {
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
          const commitToMountMs =
            navigation.committedMs !== null && navigation.hydratedMs !== null
              ? navigation.hydratedMs - navigation.committedMs
              : null;
          const domContentLoadedToLoadMs =
            navigation.domContentLoadedMs !== null && navigation.loadMs !== null
              ? navigation.loadMs - navigation.domContentLoadedMs
              : null;
          return {
            id: navigation.id,
            url: navigation.url,
            cacheWarmth: navigation.id === 1 ? "cold" : "warm",
            totalMs: endMs - navigation.startedMs,
            requestMs,
            commitToDomContentLoadedMs,
            commitToMountMs,
            domContentLoadedToLoadMs,
            requestFailed: navigation.requestFailed,
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
        makeEvent(
          "navigation.lifecycle",
          endMs,
          [
            otlpAttribute("navigation.id", navigation.id),
            otlpAttribute("url.full", navigation.url),
            otlpAttribute("navigation.cache_warmth", navigation.cacheWarmth),
            otlpAttribute("duration_ms", navigation.totalMs),
            otlpAttribute("navigation.request_ms", navigation.requestMs),
            otlpAttribute(
              "navigation.commit_to_domcontentloaded_ms",
              navigation.commitToDomContentLoadedMs,
            ),
            otlpAttribute(
              "navigation.commit_to_mount_ms",
              navigation.commitToMountMs,
            ),
            otlpAttribute(
              "navigation.domcontentloaded_to_load_ms",
              navigation.domContentLoadedToLoadMs,
            ),
            otlpAttribute(
              "navigation.request_failed",
              navigation.requestFailed,
            ),
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

/**
 * Client-side trace capture, attached once per browser context (#794).
 *
 * This was inlined in `fixtures.ts`'s `_autoPerfSpan` and bound to the default
 * `page`, which is why extra contexts a spec opened via `tracedContext` had no
 * client-side instrumentation at all — their server requests were attributed via
 * traceparent, but their navigation, resource and long-task cost was invisible.
 * Attaching at the *context* level, from one place, means every page in every
 * context is instrumented through one code path, including pages a spec opens
 * later via `context.newPage()`. There is no per-page opt-in left to forget.
 *
 * ## Capture is not attribution
 *
 * `_autoPerfSpan` used to attach this instrumentation *after* the per-test warmup
 * (removed in #792). The reason recorded in the comment there was about the
 * **traceparent**: pre-test traffic must stay unattributed, per #681's
 * orphan-bucket design. Capture had no such constraint — it simply inherited the
 * ordering, and that incidental fusion is why the warmup's duration was measured
 * nowhere, which is what #794 separated.
 *
 * The two remain separate: capture attaches at context creation, the traceparent
 * is applied only once the test proper begins. #681's contract is untouched.
 *
 * ## Why records are phase-tagged at `request`, not at completion
 *
 * The `pretest` phase covers everything between context creation and the
 * traceparent switch. Since #792 removed the warmup that phase is normally empty,
 * but it is not vestigial: it is what guarantees that anything a fixture does
 * before the test body cannot silently land in `e2e.test`'s arrays, where it
 * would shift `e2e.request_count` / `e2e.navigation_count` and the top-N blobs
 * and break comparability with published numbers.
 *
 * Routing on the `requestfinished` event would not be enough: a page can kick off
 * wasm/JS fetches it does not await, so a request *started* before the switch can
 * *finish* after it and would be misfiled. The phase is therefore captured when
 * the request STARTS and the completion handler files by that tag. Navigations
 * were always safe here — they are recorded on `request` — but they go through
 * the same tagging for consistency.
 */

import type { BrowserContext, Page, Request } from "@playwright/test";
import { MOUNTED_ATTR } from "./mount";

export type RequestRecord = {
  method: string;
  url: string;
  resourceType: string;
  startedMs: number;
  endedMs: number;
  durationMs: number;
  failed: boolean;
  failureText?: string;
};

export type NavigationRecord = {
  id: number;
  url: string;
  startedMs: number;
  committedMs: number | null;
  domContentLoadedMs: number | null;
  loadMs: number | null;
  mountedMs: number | null;
  requestFinishedMs: number | null;
  requestFailed: boolean;
  requestFailureText?: string;
};

export type PagePerfSummary = {
  navigation: {
    domContentLoadedMs: number;
    loadMs: number;
    responseStartMs: number;
  } | null;
  resources: {
    count: number;
    totalDurationMs: number;
    topSlow: Array<{ name: string; initiatorType: string; durationMs: number }>;
    droppedCount: number;
  };
  longTasks: Array<{ startTime: number; duration: number; name: string }>;
  longTasksDroppedCount: number;
};

/** Which lifecycle phase a record belongs to. */
export type Phase = "pretest" | "test";

export type CaptureSink = {
  requests: RequestRecord[];
  navigations: NavigationRecord[];
};

/** One `performance.mark` the CSR client emitted, document-relative. */
export type BootMark = { name: string; startTime: number };

/** The `.wasm` resource's timing within one document, document-relative. */
export type WasmTiming = {
  startTime: number;
  durationMs: number;
  responseEndMs: number;
};

/**
 * Everything harvested from a single document, at mount-ready and again at `load`.
 *
 * Harvested at BOTH points (#818). Mount-ready is the complete one by construction —
 * `csr` emits every `jaunder.*` mark synchronously before setting `data-mounted` — but
 * a navigation that never mounts still reaches `load`, and its wasm resource timing is
 * worth keeping. The two are reconciled by [`mergeDocumentTiming`].
 */
export type DocumentTiming = {
  marks: BootMark[];
  wasm: WasmTiming | null;
};

/**
 * Pick the more complete of two harvests of the same document.
 *
 * Marks persist for a document's lifetime, so a later harvest is a superset of an
 * earlier one — but `documentTimings.set` is last-*resolution*-wins, which coincides
 * with issue order only because two `page.evaluate`s on one page serialize over a
 * single connection. That is undocumented transport behavior, and firefox is exactly
 * the case that depends on it: `load` fires BEFORE mount there, so its empty snapshot
 * would win under any rule that trusts arrival order. Comparing mark counts makes the
 * invariant local (#818).
 *
 * Picks a snapshot, never blends one — a caller may rely on identity.
 */
export function mergeDocumentTiming(
  existing: DocumentTiming | undefined,
  incoming: DocumentTiming,
): DocumentTiming {
  if (existing === undefined) return incoming;
  if (incoming.marks.length !== existing.marks.length) {
    return incoming.marks.length > existing.marks.length ? incoming : existing;
  }
  // Equal mark counts: prefer whichever actually saw the `.wasm` resource entry.
  // `load` can fire before the fetch completes (that is defect 2 of #818), so a tie
  // on marks does not mean a tie on completeness.
  if (existing.wasm === null && incoming.wasm !== null) return incoming;
  return existing;
}

export type TraceCapture = {
  /** Route records that START after this call to `phase`. */
  setPhase(phase: Phase): void;
  sinkFor(phase: Phase): CaptureSink;
  /** Read the client-side perf summary. Must be called while `page` is alive. */
  readPagePerf(page: Page): Promise<PagePerfSummary>;
  /**
   * Per-document timing for `navigationId`, harvested at mount-ready and again at
   * that document's `load`.
   *
   * Harvested per navigation, NOT at teardown, because `performance` marks and
   * resource entries are per-document — a full navigation wipes them. A single
   * read at teardown could only ever see the last document, while the whole point
   * is to decompose EVERY navigation's boot (#794).
   *
   * **What this decomposes is the DOCUMENT-relative boot total** — the interval
   * from `performance.timeOrigin` to the last boot mark — not `commit_to_mount`.
   * The two are different clocks: `commitToMountMs` is built from Node-side
   * `Date.now()` stamps, so the difference between them is event-delivery latency
   * plus the mount→binding round trip, both cross-process and plausibly
   * engine-asymmetric. #794 framed the goal as decomposing `commit_to_mount`;
   * doing that would charge harness overhead to app boot phases (#818).
   */
  timingFor(navigationId: number): DocumentTiming | undefined;
  /** Await the in-flight per-document harvests. Call before reading. */
  settle(): Promise<void>;
};

/** Marks are discovered by PREFIX, never by an enumerated list of names. The
 *  names live only in Rust (`client::perf`), so adding one needs no change here —
 *  the property that keeps the two sides from drifting the way `MOUNTED_ATTR`
 *  can. */
const MARK_PREFIX = "jaunder.";

const TOP_SLOW_RESOURCE_LIMIT = 20;
const LONG_TASK_LIMIT = 20;

/** Per-page navigation bookkeeping. A context can hold several pages, and each
 *  tracks its own in-flight navigation independently. */
type PageState = {
  pending: number[];
  active: number | null;
};

export async function attachTraceCapture(
  context: BrowserContext,
): Promise<TraceCapture> {
  const sinks: Record<Phase, CaptureSink> = {
    pretest: { requests: [], navigations: [] },
    test: { requests: [], navigations: [] },
  };
  let phase: Phase = "pretest";

  const requestStarts = new Map<Request, number>();
  const requestPhase = new Map<Request, Phase>();
  const navigationRequestIds = new Map<Request, number>();
  const navigationPhase = new Map<number, Phase>();
  const pageStates = new Map<Page, PageState>();
  const documentTimings = new Map<number, DocumentTiming>();
  const pendingHarvests: Promise<void>[] = [];
  let nextNavigationId = 1;

  /**
   * Snapshot this document's marks and `.wasm` resource timing before the next
   * navigation wipes them.
   *
   * **Called at two points, and the mount-ready one is what makes this complete.**
   * `csr` emits every `jaunder.*` mark synchronously before setting `data-mounted`,
   * so a harvest driven by that attribute catches the whole set by construction on
   * any engine. The `load` harvest is kept because a navigation that never mounts
   * still reaches it and still has wasm timing worth recording — but `load` alone
   * is not enough twice over: it frequently never fires at all (`goto` waits only
   * for `domcontentloaded`), and on firefox it lands before boot has even reached
   * `boot.entry`, because `csr/index.html` starts the wasm from a module script
   * that never awaits `init(...)`, so the fetch does not block `load`. Firefox lost
   * that race on 210/210 navigations of every run in the #792 corpus (#818).
   *
   * Everything returned is document-relative (`performance.timeOrigin`-based), so
   * the values are comparable to each other but NOT to the Node-side `Date.now()`
   * fields on `NavigationRecord`. The boot decomposition is computed entirely
   * within this frame of reference; `mount_to_settled_ms` is computed entirely
   * within the Node one. The two are never mixed.
   */
  const harvestDocument = async (page: Page, navigationId: number) => {
    try {
      const timing = await page.evaluate((prefix: string) => {
        const marks = performance
          .getEntriesByType("mark")
          .filter((entry) => entry.name.startsWith(prefix))
          .map((entry) => ({ name: entry.name, startTime: entry.startTime }));
        const wasmEntry = (
          performance.getEntriesByType(
            "resource",
          ) as PerformanceResourceTiming[]
        )
          .filter((entry) => entry.name.endsWith(".wasm"))
          .sort((left, right) => left.startTime - right.startTime)[0];
        return {
          marks,
          wasm: wasmEntry
            ? {
                startTime: wasmEntry.startTime,
                durationMs: wasmEntry.duration,
                responseEndMs: wasmEntry.responseEnd,
              }
            : null,
        };
      }, MARK_PREFIX);
      // Merge, never overwrite: this runs twice per navigation (mount-ready and
      // `load`) and the two can resolve in either order. See `mergeDocumentTiming`.
      documentTimings.set(
        navigationId,
        mergeDocumentTiming(documentTimings.get(navigationId), timing),
      );
    } catch {
      // Page closed, or navigated again before the evaluate landed. A missing
      // entry is reported as absent rather than as zeros.
    }
  };

  const stateFor = (page: Page): PageState => {
    let state = pageStates.get(page);
    if (state === undefined) {
      state = { pending: [], active: null };
      pageStates.set(page, state);
    }
    return state;
  };

  /** Look a navigation up across both phases — a navigation started before the
   *  switch can still be committing when the phase changes. */
  const findNavigation = (id: number): NavigationRecord | undefined => {
    const known = navigationPhase.get(id);
    if (known !== undefined) {
      return sinks[known].navigations.find((entry) => entry.id === id);
    }
    return undefined;
  };

  await context.exposeBinding("__jaunderRecordMount", (source, value) => {
    if (!value || typeof value !== "object") return;
    const payload = value as { href?: unknown };
    const href = typeof payload.href === "string" ? payload.href : null;
    const nowMs = Date.now();

    // The mount-ready marker belongs to the most recent matching navigation
    // (`data-mounted` is set once per document). Search newest-first across both
    // phases, so a mount that lands just after the phase switch still attaches to
    // the navigation that caused it.
    const candidates = [
      ...sinks.pretest.navigations,
      ...sinks.test.navigations,
    ];
    for (let index = candidates.length - 1; index >= 0; index -= 1) {
      const navigation = candidates[index];
      if (navigation.mountedMs !== null) continue;
      if (href !== null && navigation.url !== href) continue;
      navigation.mountedMs = nowMs;
      // Harvest HERE, not only at `load`. This instant is complete by
      // construction — `csr` marks `boot.mount_done` immediately before setting
      // `data-mounted`, and this callback runs off the MutationObserver watching
      // that attribute. Safe to issue an `evaluate` from a binding callback: this
      // callback is not `async`, so Playwright resolves the binding at once and
      // the evaluate proceeds independently over the duplex connection — it never
      // re-enters the page's JS thread (#818).
      pendingHarvests.push(harvestDocument(source.page, navigation.id));
      return;
    }
  });

  // `addInitScript` serializes this callback into the page, so it cannot close
  // over `MOUNTED_ATTR` from module scope — the attribute name is passed as the
  // call's argument instead (#251).
  await context.addInitScript((mountedAttr: string) => {
    const globalScope = globalThis as typeof globalThis & {
      __jaunderLongTasks?: Array<{
        startTime: number;
        duration: number;
        name: string;
      }>;
      __jaunderLongTaskTotal?: number;
      __jaunderMountNotified?: boolean;
      __jaunderRecordMount?: (payload: { href: string }) => void;
    };
    globalScope.__jaunderLongTasks = [];
    globalScope.__jaunderLongTaskTotal = 0;
    globalScope.__jaunderMountNotified = false;

    const notifyMount = () => {
      if (globalScope.__jaunderMountNotified) return;
      const body = document.body;
      if (!body || !body.hasAttribute(mountedAttr)) return;
      globalScope.__jaunderMountNotified = true;
      try {
        globalScope.__jaunderRecordMount?.({ href: location.href });
      } catch {
        // Ignore cross-context bridge errors while collecting diagnostics.
      }
    };

    notifyMount();
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", notifyMount, {
        once: true,
      });
    }
    try {
      const mountObserver = new MutationObserver(() => notifyMount());
      mountObserver.observe(document.documentElement, {
        subtree: true,
        attributes: true,
        attributeFilter: [mountedAttr],
      });
    } catch {
      // MutationObserver should always exist in browsers, but keep this defensive.
    }

    if (typeof PerformanceObserver === "undefined") return;
    try {
      const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          if (entry.entryType !== "longtask") continue;
          globalScope.__jaunderLongTaskTotal =
            (globalScope.__jaunderLongTaskTotal ?? 0) + 1;
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
      // Note: Gecko implements no `longtask` observer at all, so this list is
      // always empty on Firefox — an engine limitation, not a capture bug.
    }
  }, MOUNTED_ATTR);

  context.on("request", (request) => {
    requestStarts.set(request, Date.now());
    // Tag at START — see the module header. A pre-test request that finishes
    // after the phase switch must still be filed under `pretest`.
    requestPhase.set(request, phase);

    const frame = request.frame();
    const isMainFrame = frame !== null && frame.parentFrame() === null;
    if (
      request.isNavigationRequest() &&
      request.resourceType() === "document" &&
      isMainFrame
    ) {
      const navigationId = nextNavigationId;
      nextNavigationId += 1;
      navigationRequestIds.set(request, navigationId);
      navigationPhase.set(navigationId, phase);
      const page = frame.page();
      if (page !== null) stateFor(page).pending.push(navigationId);
      sinks[phase].navigations.push({
        id: navigationId,
        url: request.url(),
        startedMs: Date.now(),
        committedMs: null,
        domContentLoadedMs: null,
        loadMs: null,
        mountedMs: null,
        requestFinishedMs: null,
        requestFailed: false,
      });
    }
  });

  const recordCompletion = (request: Request, failed: boolean) => {
    const startedMs = requestStarts.get(request) ?? Date.now();
    const endedMs = Date.now();
    const failureText = failed ? request.failure()?.errorText : undefined;
    sinks[requestPhase.get(request) ?? phase].requests.push({
      method: request.method(),
      url: request.url(),
      resourceType: request.resourceType(),
      startedMs,
      endedMs,
      durationMs: endedMs - startedMs,
      failed,
      ...(failureText !== undefined ? { failureText } : {}),
    });

    const navigationId = navigationRequestIds.get(request);
    if (navigationId === undefined) return;
    const navigation = findNavigation(navigationId);
    if (!navigation) return;
    navigation.requestFinishedMs = endedMs;
    navigation.url = request.url();
    if (failed) {
      navigation.requestFailed = true;
      navigation.requestFailureText = failureText;
    }
  };

  context.on("requestfinished", (request) => recordCompletion(request, false));
  context.on("requestfailed", (request) => recordCompletion(request, true));

  // `framenavigated` / `domcontentloaded` / `load` have no context-level
  // equivalent, so they are hooked per page. `context.on("page")` fires only for
  // pages created AFTER this call — and `_autoPerfSpan` depends on the `page`
  // fixture, so the default page already exists by now. Seeding over
  // `context.pages()` is therefore required, not belt-and-braces: without it the
  // test's own navigations are never committed and `navigation_count` reads 0.
  const attachPage = (page: Page) => {
    const state = stateFor(page);

    page.on("framenavigated", (frame) => {
      if (frame !== page.mainFrame()) return;
      const navigationId = state.pending.shift() ?? null;
      if (navigationId === null) return;
      state.active = navigationId;
      const navigation = findNavigation(navigationId);
      if (!navigation) return;
      navigation.committedMs = Date.now();
      navigation.url = frame.url();
    });

    page.on("domcontentloaded", () => {
      if (state.active === null) return;
      const navigation = findNavigation(state.active);
      if (!navigation) return;
      if (navigation.domContentLoadedMs === null) {
        navigation.domContentLoadedMs = Date.now();
      }
    });

    page.on("load", () => {
      if (state.active === null) return;
      const navigationId = state.active;
      const navigation = findNavigation(navigationId);
      if (!navigation) return;
      if (navigation.loadMs === null) {
        navigation.loadMs = Date.now();
      }
      // Harvest THIS document's marks now — the next navigation clears them.
      pendingHarvests.push(harvestDocument(page, navigationId));
    });
  };

  context.pages().forEach(attachPage);
  context.on("page", attachPage);

  return {
    setPhase(next: Phase) {
      phase = next;
    },
    sinkFor(which: Phase) {
      return sinks[which];
    },
    timingFor(navigationId: number) {
      return documentTimings.get(navigationId);
    },
    async settle() {
      // Drain until the queue stops growing, rather than awaiting one snapshot of
      // it. `Promise.all` captures the array's contents synchronously, so anything
      // pushed WHILE we await is never awaited. That was harmless while every
      // harvest came from a `load` handler that had already fired; the mount-ready
      // harvest arrives via async binding dispatch and can land after settling has
      // begun, and a harvest missed here reads as a navigation with no boot
      // decomposition — indistinguishable from the bug this all fixes (#818).
      let drained = 0;
      while (drained < pendingHarvests.length) {
        const batch = pendingHarvests.slice(drained);
        drained = pendingHarvests.length;
        await Promise.all(batch);
      }
    },
    async readPagePerf(page: Page): Promise<PagePerfSummary> {
      const empty: PagePerfSummary = {
        navigation: null,
        resources: {
          count: 0,
          totalDurationMs: 0,
          topSlow: [],
          droppedCount: 0,
        },
        longTasks: [],
        longTasksDroppedCount: 0,
      };
      try {
        return await page.evaluate(
          ({ resourceLimit, longTaskLimit }) => {
            const navigation = performance.getEntriesByType("navigation")[0] as
              | PerformanceNavigationTiming
              | undefined;
            const resources = performance.getEntriesByType(
              "resource",
            ) as PerformanceResourceTiming[];
            const scope = globalThis as typeof globalThis & {
              __jaunderLongTasks?: Array<{
                startTime: number;
                duration: number;
                name: string;
              }>;
              __jaunderLongTaskTotal?: number;
            };
            const allLongTasks = scope.__jaunderLongTasks ?? [];
            const longTaskTotal =
              scope.__jaunderLongTaskTotal ?? allLongTasks.length;
            // A TAIL slice: the EARLIEST long tasks are the ones dropped, and
            // nothing else records that they existed.
            const longTasks = allLongTasks.slice(-longTaskLimit);

            const topSlow = resources
              .map((resource) => ({
                name: resource.name,
                initiatorType: resource.initiatorType,
                durationMs: resource.duration,
              }))
              .sort((left, right) => right.durationMs - left.durationMs)
              .slice(0, resourceLimit);

            const totalDurationMs = resources.reduce(
              (sum, resource) => sum + resource.duration,
              0,
            );

            return {
              navigation: navigation
                ? {
                    domContentLoadedMs:
                      navigation.domContentLoadedEventEnd -
                      navigation.startTime,
                    loadMs: navigation.loadEventEnd - navigation.startTime,
                    responseStartMs:
                      navigation.responseStart - navigation.startTime,
                  }
                : null,
              resources: {
                count: resources.length,
                totalDurationMs,
                topSlow,
                droppedCount: Math.max(0, resources.length - topSlow.length),
              },
              longTasks,
              longTasksDroppedCount: Math.max(
                0,
                longTaskTotal - longTasks.length,
              ),
            };
          },
          {
            resourceLimit: TOP_SLOW_RESOURCE_LIMIT,
            longTaskLimit: LONG_TASK_LIMIT,
          },
        );
      } catch {
        // Page may already be closed on failure paths.
        return empty;
      }
    },
  };
}

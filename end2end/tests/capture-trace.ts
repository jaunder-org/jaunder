/**
 * Client-side trace capture, attached once per browser context (#794).
 *
 * Attaching at the *context* level, from one place, means every page in every
 * context is instrumented through one code path — including extra contexts a
 * spec opens via `tracedContext` and pages opened later via
 * `context.newPage()`. There is no per-page opt-in left to forget; page-bound
 * instrumentation leaves those pages' navigation, resource and long-task cost
 * invisible.
 *
 * ## Capture is not attribution
 *
 * Capture attaches at context creation; the traceparent is applied only once
 * the test proper begins — pre-test traffic must stay unattributed, per #681's
 * orphan-bucket design. The two are deliberately separate concerns (#794).
 *
 * ## Why records are phase-tagged at `request`, not at completion
 *
 * The `pretest` phase covers everything between context creation and the
 * traceparent switch. That phase is normally empty (#792),
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

/** A browser request observed at its start, before completion or page teardown. */
export type RequestStartRecord = {
  method: string;
  url: string;
  resourceType: string;
  sequence: number;
  startedMs: number;
};

/** A browser console warning observed while its page was alive. */
export type ConsoleWarningRecord = {
  text: string;
  sequence: number;
  emittedMs: number;
};

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
  requestStarts: RequestStartRecord[];
  requests: RequestRecord[];
  navigations: NavigationRecord[];
  consoleWarnings: ConsoleWarningRecord[];
};

/** One `performance.mark` the CSR client emitted, document-relative. */
export type BootMark = {
  name: string;
  startTime: number;
  detail?: unknown;
};

/** The `.wasm` resource's timing within one document, document-relative.
 *
 *  The three sizes are what distinguishes "this engine received brotli" from
 *  "this engine received identity" — a confound worth ruling out before reading
 *  anything into a fetch-duration difference between browsers, since the bundle
 *  is served precompressed (`jaunder.wasm.br`, ~863 KiB against 5.35 MB raw) and
 *  only when the client asks for it. They are also the honest input to any
 *  bundle-size work: `decodedBodySize` is what the wasm compiler must chew
 *  through, `encodedBodySize` is what crosses the wire (#818/#836). */
export type WasmTiming = {
  startTime: number;
  durationMs: number;
  responseEndMs: number;
  /** Bytes after content decoding — the compiler's actual input. */
  decodedBodySize: number;
  /** Bytes of the response body as sent, i.e. post-brotli. */
  encodedBodySize: number;
  /** Bytes on the wire including headers; 0 for a cache hit. */
  transferSize: number;
};

export type WasmModuleShape = {
  imports: number;
  importedFunctions: number;
  importedTables: number;
  importedMemories: number;
  exports: number;
  exportedFunctions: number;
  exportedTables: number;
  exportedMemories: number;
  customSections: number;
};

/** Direct initializer marks and the successful WebAssembly API timing. Kept
 * separate from Rust boot marks because both direct durations overlap them. */
export type WasmInitTiming = {
  startMs: number | null;
  doneMs: number | null;
  apiMs: number | null;
  path: "streaming" | "buffered" | null;
  experimentArm: string | null;
  moduleShape: WasmModuleShape | null;
};

const WASM_INIT_START = "jaunder.wasm.init_start";
const WASM_INIT_DONE = "jaunder.wasm.init_done";
const MODULE_BEFORE_INIT = "jaunder.module.before_init";
const JAUNDER_CSS_PATH = "/style/jaunder.css";
const JAUNDER_THEMES_CSS_PATH = "/style/jaunder-themes.css";

function finiteShapeCount(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : null;
}
function finiteTimingOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function moduleShapeFromDetail(value: unknown): WasmModuleShape | null {
  if (value === null || typeof value !== "object") return null;
  const shape = value as Record<string, unknown>;
  const imports = finiteShapeCount(shape.imports);
  const importedFunctions = finiteShapeCount(shape.importedFunctions);
  const importedTables = finiteShapeCount(shape.importedTables);
  const importedMemories = finiteShapeCount(shape.importedMemories);
  const exports = finiteShapeCount(shape.exports);
  const exportedFunctions = finiteShapeCount(shape.exportedFunctions);
  const exportedTables = finiteShapeCount(shape.exportedTables);
  const exportedMemories = finiteShapeCount(shape.exportedMemories);
  const customSections = finiteShapeCount(shape.customSections);
  if (
    imports === null ||
    importedFunctions === null ||
    importedTables === null ||
    importedMemories === null ||
    exports === null ||
    exportedFunctions === null ||
    exportedTables === null ||
    exportedMemories === null ||
    customSections === null
  ) {
    return null;
  }
  return {
    imports,
    importedFunctions,
    importedTables,
    importedMemories,
    exports,
    exportedFunctions,
    exportedTables,
    exportedMemories,
    customSections,
  };
}

/** Decode only the closed completion payload the generated initializer writes. */
export function wasmInitFromMarks(
  marks: BootMark[],
): WasmInitTiming | undefined {
  const start = marks.find((mark) => mark.name === WASM_INIT_START);
  const done = marks.find((mark) => mark.name === WASM_INIT_DONE);
  if (!start && !done) return undefined;
  const detail = done?.detail;
  const completion: Record<string, unknown> | null =
    detail !== null &&
    typeof detail === "object" &&
    "path" in detail &&
    "apiMs" in detail
      ? (detail as Record<string, unknown>)
      : null;
  const candidatePath =
    completion?.path === "streaming"
      ? "streaming"
      : completion?.path === "buffered"
        ? "buffered"
        : null;
  const candidateApiMs =
    typeof completion?.apiMs === "number" && Number.isFinite(completion.apiMs)
      ? completion.apiMs
      : null;
  const candidateExperimentArm =
    typeof completion?.experimentArm === "string" &&
    completion.experimentArm.length > 0
      ? completion.experimentArm
      : null;
  const candidateModuleShape = moduleShapeFromDetail(completion?.moduleShape);
  const valid =
    candidatePath !== null && candidateApiMs !== null && candidateApiMs >= 0;
  const path = valid ? candidatePath : null;
  const apiMs = valid ? candidateApiMs : null;
  return {
    startMs: start?.startTime ?? null,
    doneMs: done?.startTime ?? null,
    apiMs,
    path,
    experimentArm: valid ? candidateExperimentArm : null,
    moduleShape: valid ? candidateModuleShape : null,
  };
}

function mergeWasmInit(
  existing: WasmInitTiming | undefined,
  incoming: WasmInitTiming | undefined,
): WasmInitTiming | undefined {
  if (existing === undefined) return incoming;
  if (incoming === undefined) return existing;
  return {
    startMs: existing.startMs ?? incoming.startMs,
    doneMs: existing.doneMs ?? incoming.doneMs,
    apiMs: existing.apiMs ?? incoming.apiMs,
    path: existing.path ?? incoming.path,
    experimentArm: existing.experimentArm ?? incoming.experimentArm,
    moduleShape: existing.moduleShape ?? incoming.moduleShape,
  };
}
function mergeFiniteTiming(
  existing: number | null | undefined,
  incoming: number | null | undefined,
): number | null {
  return finiteTimingOrNull(existing) ?? finiteTimingOrNull(incoming);
}

/**
 * Everything harvested from a single document, at mount-ready and again at `load`.
 *
 * Harvested at BOTH points (#818). Mount-ready is the complete one by construction —
 * `csr` emits every `jaunder.*` mark synchronously before setting `data-mounted` — but
 * a navigation that never mounts still reaches `load`, and its wasm resource timing is
 * worth keeping. The two are reconciled by [`mergeDocumentTiming`].
 */
export type DocumentTiming = {
  timeOriginMs: number | null;
  marks: BootMark[];
  moduleBeforeInitMs: number | null;
  jaunderCssResponseEndMs: number | null;
  jaunderThemesCssResponseEndMs: number | null;
  wasm: WasmTiming | null;
  /** Completion can arrive after mount/load snapshots; merge independently. */
  wasmInit?: WasmInitTiming;
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
 * Picks the more complete snapshot for marks/wasm, then backfills missing scalar
 * diagnostics from its sibling harvest.
 */
export function mergeDocumentTiming(
  existing: DocumentTiming | undefined,
  incoming: DocumentTiming,
): DocumentTiming {
  if (existing === undefined) return incoming;
  let selected = existing;
  if (incoming.marks.length !== existing.marks.length) {
    selected =
      incoming.marks.length > existing.marks.length ? incoming : existing;
  } else if (existing.wasm === null && incoming.wasm !== null) {
    selected = incoming;
  }
  const timeOriginMs =
    existing.timeOriginMs ??
    (typeof incoming.timeOriginMs === "number" &&
    Number.isFinite(incoming.timeOriginMs)
      ? incoming.timeOriginMs
      : null);
  const moduleBeforeInitMs = mergeFiniteTiming(
    existing.moduleBeforeInitMs,
    incoming.moduleBeforeInitMs,
  );
  const jaunderCssResponseEndMs = mergeFiniteTiming(
    existing.jaunderCssResponseEndMs,
    incoming.jaunderCssResponseEndMs,
  );
  const jaunderThemesCssResponseEndMs = mergeFiniteTiming(
    existing.jaunderThemesCssResponseEndMs,
    incoming.jaunderThemesCssResponseEndMs,
  );
  const wasmInit = mergeWasmInit(existing.wasmInit, incoming.wasmInit);
  if (
    (wasmInit === undefined || selected.wasmInit === wasmInit) &&
    selected.timeOriginMs === timeOriginMs &&
    selected.moduleBeforeInitMs === moduleBeforeInitMs &&
    selected.jaunderCssResponseEndMs === jaunderCssResponseEndMs &&
    selected.jaunderThemesCssResponseEndMs === jaunderThemesCssResponseEndMs
  ) {
    return selected;
  }
  return {
    ...selected,
    timeOriginMs,
    moduleBeforeInitMs,
    jaunderCssResponseEndMs,
    jaunderThemesCssResponseEndMs,
    wasmInit,
  };
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
 *  names live only in Rust (`client::perf`), so adding one needs no change here:
 *  the mark *names* cannot drift, because there is only ever one copy of them.
 *
 *  The prefix is the exception — it is spelled here and in
 *  `client::perf::MARK_PREFIX`, because no import crosses into Node. That one
 *  duplication is checked by the `xlang-literal` gate
 *  (`xtask/src/steps/xlang_literal_check.rs`), which also covers `MOUNTED_ATTR`
 *  and its Rust counterpart (#767). */
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
    pretest: {
      requestStarts: [],
      requests: [],
      navigations: [],
      consoleWarnings: [],
    },
    test: {
      requestStarts: [],
      requests: [],
      navigations: [],
      consoleWarnings: [],
    },
  };
  let phase: Phase = "pretest";

  const requestStartedMs = new Map<Request, number>();
  const requestPhase = new Map<Request, Phase>();
  const navigationRequestIds = new Map<Request, number>();
  const navigationPhase = new Map<number, Phase>();
  const pageStates = new Map<Page, PageState>();
  const documentTokens = new Map<number, number>();
  const documentTimings = new Map<number, DocumentTiming>();
  const pendingHarvests: Promise<void>[] = [];
  let nextNavigationId = 1;
  let nextRecordSequence = 1;
  /**
   * Harvest at mount-ready and load, then reconcile with the initializer
   * completion callback. `load` can precede both boot and wasm completion.
   */
  const harvestDocument = async (page: Page, navigationId: number) => {
    try {
      const timing = await page.evaluate(
        ({
          prefix,
          moduleBeforeInitMark,
          jaunderCssPath,
          jaunderThemesCssPath,
        }) => {
          const marks = performance
            .getEntriesByType("mark")
            .filter((entry) => entry.name.startsWith(prefix))
            .map((entry) => {
              // `getEntriesByType` erases the concrete mark type.
              const mark = entry as PerformanceMark;
              return {
                name: mark.name,
                startTime: mark.startTime,
                detail: mark.detail,
              };
            });
          const resources = performance.getEntriesByType(
            "resource",
          ) as PerformanceResourceTiming[];
          const resourceForPath = (pathname: string) =>
            resources
              .filter((entry) => {
                try {
                  return (
                    new URL(entry.name, location.href).pathname === pathname
                  );
                } catch {
                  return false;
                }
              })
              .sort((left, right) => left.startTime - right.startTime)[0];
          const wasmEntry = resources
            .filter((entry) => entry.name.endsWith(".wasm"))
            .sort((left, right) => left.startTime - right.startTime)[0];
          return {
            timeOriginMs: performance.timeOrigin,
            marks,
            moduleBeforeInitMs:
              marks.find((mark) => mark.name === moduleBeforeInitMark)
                ?.startTime ?? null,
            jaunderCssResponseEndMs:
              resourceForPath(jaunderCssPath)?.responseEnd ?? null,
            jaunderThemesCssResponseEndMs:
              resourceForPath(jaunderThemesCssPath)?.responseEnd ?? null,
            wasm: wasmEntry
              ? {
                  startTime: wasmEntry.startTime,
                  durationMs: wasmEntry.duration,
                  responseEndMs: wasmEntry.responseEnd,
                  decodedBodySize: wasmEntry.decodedBodySize,
                  encodedBodySize: wasmEntry.encodedBodySize,
                  transferSize: wasmEntry.transferSize,
                }
              : null,
          };
        },
        {
          prefix: MARK_PREFIX,
          moduleBeforeInitMark: MODULE_BEFORE_INIT,
          jaunderCssPath: JAUNDER_CSS_PATH,
          jaunderThemesCssPath: JAUNDER_THEMES_CSS_PATH,
        },
      );
      const harvested: DocumentTiming = {
        ...timing,
        timeOriginMs:
          typeof timing.timeOriginMs === "number" &&
          Number.isFinite(timing.timeOriginMs)
            ? timing.timeOriginMs
            : null,
        moduleBeforeInitMs: finiteTimingOrNull(timing.moduleBeforeInitMs),
        jaunderCssResponseEndMs: finiteTimingOrNull(
          timing.jaunderCssResponseEndMs,
        ),
        jaunderThemesCssResponseEndMs: finiteTimingOrNull(
          timing.jaunderThemesCssResponseEndMs,
        ),
        wasmInit: wasmInitFromMarks(timing.marks),
      };
      documentTimings.set(
        navigationId,
        mergeDocumentTiming(documentTimings.get(navigationId), harvested),
      );
    } catch {
      // Page closed, or navigated again before the evaluate landed.
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
    const payload = value as { href?: unknown; token?: unknown };
    const href = typeof payload.href === "string" ? payload.href : null;
    const token = typeof payload.token === "number" ? payload.token : null;
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
      if (token !== null) documentTokens.set(token, navigation.id);
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

  await context.exposeBinding("__jaunderRecordWasmInit", (source, value) => {
    if (!value || typeof value !== "object") return;
    const payload = value as { href?: unknown; token?: unknown };
    const href = typeof payload.href === "string" ? payload.href : null;
    const token = typeof payload.token === "number" ? payload.token : null;
    if (href === null) return;
    const tokenNavigationId =
      token === null ? undefined : documentTokens.get(token);
    const active = stateFor(source.page).active;
    const navigation =
      tokenNavigationId === undefined
        ? active === null
          ? undefined
          : findNavigation(active)
        : findNavigation(tokenNavigationId);
    if (navigation !== undefined && navigation.url === href) {
      pendingHarvests.push(harvestDocument(source.page, navigation.id));
    }
  });

  // `addInitScript` serializes this callback into the page, so it cannot close
  // over `MOUNTED_ATTR` from module scope — the attribute name is passed as the
  // call's argument instead (#251).
  await context.addInitScript(
    (args: { mountedAttr: string; wasmInitDone: string }) => {
      const { mountedAttr, wasmInitDone } = args;
      const globalScope = globalThis as typeof globalThis & {
        __jaunderLongTasks?: Array<{
          startTime: number;
          duration: number;
          name: string;
        }>;
        __jaunderLongTaskTotal?: number;
        __jaunderMountNotified?: boolean;
        __jaunderRecordMount?: (payload: {
          href: string;
          token: number;
        }) => void;
        __jaunderRecordWasmInit?: (payload: {
          href: string;
          token: number;
        }) => void;
      };
      globalScope.__jaunderLongTasks = [];
      globalScope.__jaunderLongTaskTotal = 0;
      globalScope.__jaunderMountNotified = false;
      const documentToken = performance.timeOrigin;

      const notifyMount = () => {
        if (globalScope.__jaunderMountNotified) return;
        const body = document.body;
        if (!body || !body.hasAttribute(mountedAttr)) return;
        globalScope.__jaunderMountNotified = true;
        try {
          globalScope.__jaunderRecordMount?.({
            href: location.href,
            token: documentToken,
          });
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

      try {
        const observer = new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            if (entry.name !== wasmInitDone) continue;
            try {
              globalScope.__jaunderRecordWasmInit?.({
                href: location.href,
                token: documentToken,
              });
            } catch {
              // Ignore cross-context bridge errors while collecting diagnostics.
            }
          }
        });
        observer.observe({ type: "mark", buffered: true });
      } catch {
        // PerformanceObserver can be absent in reduced browser environments.
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
    },
    { mountedAttr: MOUNTED_ATTR, wasmInitDone: WASM_INIT_DONE },
  );

  context.on("request", (request) => {
    const startedMs = Date.now();
    requestStartedMs.set(request, startedMs);
    // Tag and expose the request at START. Keep this separate from completion:
    // keepalive diagnostics may outlive their page, and the browser contract only
    // guarantees that delivery starts before teardown.
    requestPhase.set(request, phase);
    sinks[phase].requestStarts.push({
      method: request.method(),
      url: request.url(),
      resourceType: request.resourceType(),
      startedMs,
      sequence: nextRecordSequence,
    });
    nextRecordSequence += 1;
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
    const startedMs = requestStartedMs.get(request) ?? Date.now();
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
    page.on("console", (message) => {
      if (message.type() !== "warning") return;
      sinks[phase].consoleWarnings.push({
        text: message.text(),
        emittedMs: Date.now(),
        sequence: nextRecordSequence,
      });
      nextRecordSequence += 1;
    });

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
      // pushed WHILE we await is never awaited — and the mount-ready harvest
      // arrives via async binding dispatch, so it can land after settling has
      // begun. A harvest missed here reads as a navigation with no boot
      // decomposition — indistinguishable from the bug this all fixes (#818).
      // Give bindings queued by the page's last synchronous turn a bounded
      // opportunity to arrive. In particular, `init_done` is emitted after
      // mount while the initializer promise resolves; absent/hung initializers
      // still cannot keep settlement open.
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 50);
      });
      const incompleteInitializers = [...pageStates].flatMap(
        ([page, state]) => {
          if (state.active === null) return [];
          const doneMs = documentTimings.get(state.active)?.wasmInit?.doneMs;
          return doneMs === null || doneMs === undefined
            ? [[state.active, page] as const]
            : [];
        },
      );
      await Promise.all(
        incompleteInitializers.map(async ([navigationId, page]) => {
          try {
            await page.waitForFunction(
              (name) => performance.getEntriesByName(name, "mark").length !== 0,
              WASM_INIT_DONE,
              { timeout: 500 },
            );
          } catch {
            // Failed and hung initializers do not block trace settlement.
          }
          pendingHarvests.push(harvestDocument(page, navigationId));
        }),
      );
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

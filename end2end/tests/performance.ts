/**
 * E2E Performance/OTel lifecycle infrastructure.
 *
 * Owns trace propagation, capture handoff, navigation telemetry, and the
 * fixture-lifecycle envelope. Capture attaches once per browser context and
 * records remain attributed to their originating test span; `fixtures.ts`
 * explicitly composes this behavior into the suite's test surface.
 */

import type { Browser, BrowserContext, Page, TestInfo } from "@playwright/test";
import {
  drainActionsForTest,
  setCurrentActionTestKey,
  type ActionRecord,
} from "./actions";
import {
  attachTraceCapture,
  type BootMark,
  type BrowserDiagnosticRecord,
  type CaptureSink,
  type DocumentTiming,
  type NavigationRecord,
  type PagePerfSummary,
  type RequestRecord,
  type WasmModuleShape,
  type TraceCapture,
} from "./capture-trace";
import { takeBudgetFailures, trackBoots } from "./bootBudget";
import {
  buildSpan,
  exportSpans,
  makeEvent,
  newSpanId,
  otlpAttribute,
  traceContextFromEnvironment,
} from "./otel";

type Use<T> = (value: T) => Promise<void>;
type AutoFixture<Args, T> = [
  (args: Args, use: Use<T>, testInfo: TestInfo) => Promise<void>,
  { auto: true },
];

export type NewTracedContext = (
  options?: Parameters<Browser["newContext"]>[0],
) => Promise<BrowserContext>;

type TracedContextRecord = {
  capture: TraceCapture;
  perf: PagePerfSummary | null;
};

const tracedContextRecords = new Map<string, TracedContextRecord[]>();

const BROWSER_DIAGNOSTIC_LIMIT = 20;

/** Serialize the earliest browser diagnostics without capping the raw sink. */
export function browserDiagnosticTelemetryFrom(
  diagnostics: BrowserDiagnosticRecord[],
): { json: string; dropped: number } {
  const exported = diagnostics.slice(0, BROWSER_DIAGNOSTIC_LIMIT);
  return {
    json: JSON.stringify(exported),
    dropped: diagnostics.length - exported.length,
  };
}

/** The shared browser-diagnostic schema for `e2e.test` and `e2e.page`. */
export function browserDiagnosticAttributesFrom(
  diagnostics: BrowserDiagnosticRecord[],
) {
  const telemetry = browserDiagnosticTelemetryFrom(diagnostics);
  return [
    otlpAttribute("e2e.console_json", telemetry.json),
    otlpAttribute("e2e.console_dropped", telemetry.dropped),
  ];
}
const captureByTestSpanId = new Map<string, TraceCapture>();
const captureByTracedContext = new WeakMap<BrowserContext, TraceCapture>();

/** Return the capture already attached by `tracedContext`, if this fixture made it. */
export function tracedContextCapture(
  context: BrowserContext,
): TraceCapture | undefined {
  return captureByTracedContext.get(context);
}

/**
 * The diagnostics projected onto one of the two span owners that carries browser
 * output. Keeping the owner beside its attributes makes their shared routing
 * explicit without widening either span's time range.
 */
export function browserDiagnosticSpanProjectionFor(
  spanName: "e2e.test" | "e2e.page",
  capture: TraceCapture,
) {
  return {
    spanName,
    attributes: browserDiagnosticAttributesFrom(
      capture.sinkFor("test").browserDiagnostics,
    ),
  };
}

export type NavigationSummary = {
  id: number;
  url: string;
  cacheWarmth: "cold" | "warm";
  totalMs: number;
  requestMs: number | null;
  commitToDomContentLoadedMs: number | null;
  commitToMountMs: number | null;
  domContentLoadedToLoadMs: number | null;
  requestFailed: boolean;
  /** Document-frame boot diagnostics, never a decomposition of `commitToMountMs`
   *  (see `capture-trace.ts` / ADR-0100). `null` where the document did not
   *  report the input. */
  wasmTimingSchema: "direct-init-v1";
  wasmFetchStartMs: number | null;
  moduleBeforeInitMs: number | null;
  jaunderCssResponseEndMs: number | null;
  jaunderThemesCssResponseEndMs: number | null;
  styleMaxResponseEndMs: number | null;
  styleToModuleBeforeInitMs: number | null;
  moduleBeforeInitToWasmFetchStartMs: number | null;
  wasmFetchMs: number | null;
  wasmInitStartMs: number | null;
  wasmInitStartToBootEntryMs: number | null;
  wasmApiMs: number | null;
  wasmInitMs: number | null;
  wasmInitPath: "streaming" | "buffered" | null;
  /** Arm-integrity metadata for measurement-only variants. Separate from the
   *  boot decomposition and direct wasm timing diagnostics. */
  wasmExperimentArm: string | null;
  wasmModuleShape: WasmModuleShape | null;
  /** Wasm response sizes. `decoded` is the compiler's input; `encoded` is what
   *  crossed the wire. A `decoded > encoded` pair means the engine received the
   *  precompressed `jaunder.wasm.br`; equal sizes mean identity. Recorded because
   *  a fetch-duration difference between engines is uninterpretable without
   *  knowing they were fed the same bytes (#818). */
  wasmDecodedBytes: number | null;
  wasmEncodedBytes: number | null;
  wasmTransferBytes: number | null;
  /** Bridge-only diagnostics between the Node and document time frames. */
  frameSkewSchema: "bridge-v1" | null;
  documentTimeOriginMs: number | null;
  documentBootTotalMs: number | null;
  commitToDocumentStartMs: number | null;
  mountDoneToBindingMs: number | null;
  frameSkewRemainderMs: number | null;
  bootPhases: Record<string, number> | null;
  /** Mount-ready → the last mount-path request finishing. Covers what
   *  `commitToMountMs` does NOT: `data-mounted` is set the instant
   *  `mount_to_body` returns, so the shell/route fetches resolve after it. */
  mountToSettledMs: number | null;
};
export type StylesheetModuleDiagnostics = {
  moduleBeforeInitMs: number | null;
  jaunderCssResponseEndMs: number | null;
  jaunderThemesCssResponseEndMs: number | null;
  styleMaxResponseEndMs: number | null;
  styleToModuleBeforeInitMs: number | null;
  moduleBeforeInitToWasmFetchStartMs: number | null;
};

export function stylesheetModuleDiagnosticsFrom(
  timing: DocumentTiming | undefined,
): StylesheetModuleDiagnostics {
  const moduleBeforeInitMs =
    typeof timing?.moduleBeforeInitMs === "number" &&
    Number.isFinite(timing.moduleBeforeInitMs)
      ? timing.moduleBeforeInitMs
      : null;
  const jaunderCssResponseEndMs =
    typeof timing?.jaunderCssResponseEndMs === "number" &&
    Number.isFinite(timing.jaunderCssResponseEndMs)
      ? timing.jaunderCssResponseEndMs
      : null;
  const jaunderThemesCssResponseEndMs =
    typeof timing?.jaunderThemesCssResponseEndMs === "number" &&
    Number.isFinite(timing.jaunderThemesCssResponseEndMs)
      ? timing.jaunderThemesCssResponseEndMs
      : null;
  const wasmFetchStartMs =
    typeof timing?.wasm?.startTime === "number" &&
    Number.isFinite(timing.wasm.startTime)
      ? timing.wasm.startTime
      : null;
  const styleMaxResponseEndMs =
    jaunderCssResponseEndMs !== null && jaunderThemesCssResponseEndMs !== null
      ? Math.max(jaunderCssResponseEndMs, jaunderThemesCssResponseEndMs)
      : null;
  return {
    moduleBeforeInitMs,
    jaunderCssResponseEndMs,
    jaunderThemesCssResponseEndMs,
    styleMaxResponseEndMs,
    styleToModuleBeforeInitMs:
      moduleBeforeInitMs !== null && styleMaxResponseEndMs !== null
        ? moduleBeforeInitMs - styleMaxResponseEndMs
        : null,
    moduleBeforeInitToWasmFetchStartMs:
      moduleBeforeInitMs !== null && wasmFetchStartMs !== null
        ? wasmFetchStartMs - moduleBeforeInitMs
        : null,
  };
}

/**
 * Decompose one document's boot marks into consecutive phase durations.
 *
 * Ordered by observed `startTime` rather than by an expected name sequence, so a
 * mark added in Rust that this file has never heard of still lands in the right
 * place. Returns `null` when fewer than two marks were seen — one mark yields no
 * interval, and reporting `{}` would read as "measured, all zero".
 */
function bootPhasesFrom(marks: BootMark[]): Record<string, number> | null {
  const bootMarks = marks.filter((mark) =>
    mark.name.startsWith("jaunder.boot."),
  );
  if (bootMarks.length < 2) return null;
  const sorted = [...bootMarks].sort(
    (left, right) => left.startTime - right.startTime,
  );
  const phases: Record<string, number> = {};
  for (let index = 1; index < sorted.length; index += 1) {
    const from = sorted[index - 1];
    const to = sorted[index];
    phases[`${from.name}->${to.name}`] = to.startTime - from.startTime;
  }
  return phases;
}

/**
 * The interval from mount-ready to the last mount-path request finishing, or
 * `null` when no request qualifies.
 *
 * A "mount-path request" is defined mechanically (spec D6): it starts at or after
 * the navigation committed, finishes after mount-ready, and starts BEFORE the
 * earlier of the first timed action after mount-ready or the next navigation's
 * start. That last clause is what keeps a post-mount user click's fetches from
 * being counted as app boot cost.
 *
 * KNOWN APPROXIMATION: `requests` belongs to the capture sink whose navigations
 * are being summarized, but `actions` is the whole test's — including `flow.*`
 * driven on other `tracedContext` pages. An action on an unrelated context can
 * therefore close the boundary early and truncate this window, biasing the figure
 * DOWN. Actions are not tagged with the context that ran them (`ActionRecord`
 * carries a page URL, not an identity), so closing this needs a wider change; an
 * under-estimate was preferred to over-attributing later fetches to this boot.
 */
function mountToSettledMs(
  navigation: NavigationRecord,
  nextNavigationStartedMs: number | null,
  requests: RequestRecord[],
  actions: ActionRecord[],
): number | null {
  const { committedMs, mountedMs } = navigation;
  if (committedMs === null || mountedMs === null) return null;

  const firstActionAfterMount = actions
    .filter((action) => action.startedMs >= mountedMs)
    .reduce<
      number | null
    >((earliest, action) => (earliest === null || action.startedMs < earliest ? action.startedMs : earliest), null);
  const boundary = [firstActionAfterMount, nextNavigationStartedMs]
    .filter((value): value is number => value !== null)
    .reduce<
      number | null
    >((lowest, value) => (lowest === null || value < lowest ? value : lowest), null);

  const settledMs = requests
    .filter(
      (request) =>
        request.startedMs >= committedMs &&
        request.endedMs > mountedMs &&
        (boundary === null || request.startedMs < boundary),
    )
    .reduce<
      number | null
    >((latest, request) => (latest === null || request.endedMs > latest ? request.endedMs : latest), null);

  return settledMs === null ? null : settledMs - mountedMs;
}
export type NavigationBridgeFields = {
  frameSkewSchema: "bridge-v1" | null;
  documentTimeOriginMs: number | null;
  documentBootTotalMs: number | null;
  commitToDocumentStartMs: number | null;
  mountDoneToBindingMs: number | null;
  frameSkewRemainderMs: number | null;
};

export function navigationBridgeFieldsFrom(
  navigation: Pick<NavigationRecord, "committedMs" | "mountedMs">,
  timing: DocumentTiming | undefined,
): NavigationBridgeFields {
  const committedMs =
    typeof navigation.committedMs === "number" &&
    Number.isFinite(navigation.committedMs)
      ? navigation.committedMs
      : null;
  const mountedMs =
    typeof navigation.mountedMs === "number" &&
    Number.isFinite(navigation.mountedMs)
      ? navigation.mountedMs
      : null;
  const documentTimeOriginMs =
    typeof timing?.timeOriginMs === "number" &&
    Number.isFinite(timing.timeOriginMs)
      ? timing.timeOriginMs
      : null;
  const mountDoneMark = timing?.marks.find(
    (mark) => mark.name === "jaunder.boot.mount_done",
  );
  const documentBootTotalMs =
    typeof mountDoneMark?.startTime === "number" &&
    Number.isFinite(mountDoneMark.startTime)
      ? mountDoneMark.startTime
      : null;
  if (
    committedMs === null ||
    mountedMs === null ||
    documentTimeOriginMs === null ||
    documentBootTotalMs === null
  ) {
    return {
      frameSkewSchema: null,
      documentTimeOriginMs: null,
      documentBootTotalMs: null,
      commitToDocumentStartMs: null,
      mountDoneToBindingMs: null,
      frameSkewRemainderMs: null,
    };
  }
  const commitToMountMs = mountedMs - committedMs;
  const commitToDocumentStartMs = documentTimeOriginMs - committedMs;
  const mountDoneToBindingMs =
    mountedMs - (documentTimeOriginMs + documentBootTotalMs);
  return {
    frameSkewSchema: "bridge-v1",
    documentTimeOriginMs,
    documentBootTotalMs,
    commitToDocumentStartMs,
    mountDoneToBindingMs,
    frameSkewRemainderMs:
      commitToMountMs -
      documentBootTotalMs -
      commitToDocumentStartMs -
      mountDoneToBindingMs,
  };
}

export type NavigationTopTelemetry = {
  topNavigations: NavigationSummary[];
  json: string;
  dropped: number;
};

export function navigationSummariesFrom(
  navigations: NavigationRecord[],
  requests: RequestRecord[],
  actions: ActionRecord[],
  timingFor: (navigationId: number) => DocumentTiming | undefined,
): NavigationSummary[] {
  return navigations
    .map((navigation, index): NavigationSummary => {
      const endMs =
        navigation.mountedMs ??
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
        navigation.committedMs !== null && navigation.mountedMs !== null
          ? navigation.mountedMs - navigation.committedMs
          : null;
      const domContentLoadedToLoadMs =
        navigation.domContentLoadedMs !== null && navigation.loadMs !== null
          ? navigation.loadMs - navigation.domContentLoadedMs
          : null;
      // `commitToMountMs` ends at `data-mounted`, which `csr` sets the instant
      // `mount_to_body` returns — so the shell/route fetches are NOT in it.
      // The boot marks decompose what IS in it; `mountToSettledMs` covers what
      // follows. Sizing mount cost needs both (#801).
      const timing = timingFor(navigation.id);
      const wasm = timing?.wasm ?? null;
      const bootEntry = timing?.marks.find(
        (mark) => mark.name === "jaunder.boot.entry",
      );
      const bridge = navigationBridgeFieldsFrom(navigation, timing);
      const stylesheetModule = stylesheetModuleDiagnosticsFrom(timing);
      const wasmInit = timing?.wasmInit;
      const wasmInitMs =
        wasmInit?.startMs !== null &&
        wasmInit?.startMs !== undefined &&
        wasmInit.doneMs !== null &&
        wasmInit.doneMs !== undefined &&
        wasmInit.doneMs >= wasmInit.startMs
          ? wasmInit.doneMs - wasmInit.startMs
          : null;
      // Positional, not `startedMs >`: navigations are pushed in start order, and
      // two can share a `Date.now()` millisecond. A `>` search skips the tied
      // neighbour and lands on the one after it, widening the window so a later
      // navigation's fetches get counted as this one's settle.
      const next = navigations[index + 1];
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
        wasmFetchStartMs: wasm?.startTime ?? null,
        ...stylesheetModule,
        wasmFetchMs: wasm?.durationMs ?? null,
        wasmDecodedBytes: wasm?.decodedBodySize ?? null,
        wasmEncodedBytes: wasm?.encodedBodySize ?? null,
        wasmTransferBytes: wasm?.transferSize ?? null,
        wasmTimingSchema: "direct-init-v1",
        wasmInitStartMs: wasmInit?.startMs ?? null,
        wasmInitStartToBootEntryMs:
          wasmInit?.startMs !== null &&
          wasmInit?.startMs !== undefined &&
          bootEntry !== undefined &&
          bootEntry.startTime >= wasmInit.startMs
            ? bootEntry.startTime - wasmInit.startMs
            : null,
        wasmApiMs: wasmInit?.apiMs ?? null,
        wasmInitMs,
        wasmInitPath: wasmInit?.path ?? null,
        wasmExperimentArm: wasmInit?.experimentArm ?? null,
        wasmModuleShape: wasmInit?.moduleShape ?? null,
        frameSkewSchema: bridge.frameSkewSchema,
        documentTimeOriginMs: bridge.documentTimeOriginMs,
        documentBootTotalMs: bridge.documentBootTotalMs,
        commitToDocumentStartMs: bridge.commitToDocumentStartMs,
        mountDoneToBindingMs: bridge.mountDoneToBindingMs,
        frameSkewRemainderMs: bridge.frameSkewRemainderMs,
        bootPhases: bootPhasesFrom(timing?.marks ?? []),
        mountToSettledMs: mountToSettledMs(
          navigation,
          next?.startedMs ?? null,
          requests,
          actions,
        ),
      };
    })
    .sort((left, right) => right.totalMs - left.totalMs);
}

export function navigationTopTelemetryFrom(
  navigationSummary: NavigationSummary[],
  navigationCount: number,
): NavigationTopTelemetry {
  const topNavigations = navigationSummary.slice(0, 20);
  return {
    topNavigations,
    json: JSON.stringify(topNavigations),
    dropped: navigationCount - topNavigations.length,
  };
}

/**
 * Point every request from `context` at this test's `e2e.test` span, by sending a
 * W3C `traceparent` whose parent-span-id is `testSpanId`.
 *
 * The server adopts an inbound traceparent as its request span's parent
 * (`make_request_span`), so each server request span ends up carrying this test's
 * span id — the structural join the flow-coverage gate walks (#681). Without it,
 * `playwright.config.ts` supplies one run-wide traceparent shared by every test,
 * and the suite runs `fullyParallel`, so hits could not be attributed to a test at
 * all.
 *
 * Must be called for EVERY context a test uses: `browser.newContext()` does not
 * inherit the config-level `extraHTTPHeaders`, so a throwaway context would
 * otherwise send no traceparent whatsoever and its traffic would be orphaned.
 */
export async function applyTestTraceparent(
  context: BrowserContext,
  traceId: string,
  testSpanId: string,
): Promise<void> {
  await context.setExtraHTTPHeaders({
    traceparent: `00-${traceId}-${testSpanId}-01`,
  });
}

export const lifecycleStartFixture = [
  async ({}, use: Use<number>) => {
    await use(Date.now());
  },
  { auto: true },
] satisfies AutoFixture<{}, number>;

export const testSpanIdFixture = async ({}, use: Use<string>) => {
  await use(newSpanId());
};

export const bootTimingFixture = async (
  { testSpanId }: { testSpanId: string },
  use: Use<() => Promise<DocumentTiming | undefined>>,
) => {
  await use(async () => {
    const capture = captureByTestSpanId.get(testSpanId);
    if (capture === undefined) return undefined;
    await capture.settle();
    const navigations = [
      ...capture.sinkFor("pretest").navigations,
      ...capture.sinkFor("test").navigations,
    ].filter((navigation) => navigation.mountedMs !== null);
    const newest = navigations.at(-1);
    return newest === undefined ? undefined : capture.timingFor(newest.id);
  });
};

export const browserTraceFixture = async (
  { testSpanId }: { testSpanId: string },
  use: Use<() => CaptureSink | undefined>,
) => {
  await use(() => captureByTestSpanId.get(testSpanId)?.sinkFor("test"));
};

export const tracedContextFixture = async (
  { browser, testSpanId }: { browser: Browser; testSpanId: string },
  use: Use<NewTracedContext>,
) => {
  const { traceId } = traceContextFromEnvironment();
  const opened: TracedContextRecord[] = [];
  try {
    await use(async (options) => {
      const context = await browser.newContext(options);
      await applyTestTraceparent(context, traceId, testSpanId);
      // Same instrumentation the default page gets, through the same code path —
      // an uninstrumented extra context makes a multi-context test under-report
      // its own client cost (#794).
      const capture = await attachTraceCapture(context);
      capture.setPhase("test");
      captureByTracedContext.set(context, capture);
      const record: TracedContextRecord = { capture, perf: null };
      opened.push(record);

      // Arm the boot budget on every page this context opens (#867). Wrapping
      // `newPage` here covers all 15 spec-side `newPage()` sites at once — the
      // budget's unit is the `Page`, so a second page that is never armed is a
      // blind spot, not a page exempt from the rule.
      const newPage = context.newPage.bind(context);
      context.newPage = async (...args: Parameters<typeof newPage>) => {
        const page = await newPage(...args);
        trackBoots(page);
        return page;
      };

      // Snapshot the client-side perf BEFORE the context closes. `on("close")`
      // fires *after* closing, when `page.evaluate` would throw — and the caller
      // owns this context's lifetime, so wrapping `close` is the only hook that
      // reliably runs while a page is still alive. Settle first so secondary
      // captures expose the same complete per-navigation timing as the default
      // page before any read consumes them.
      const close = context.close.bind(context);
      context.close = async (...args: Parameters<typeof close>) => {
        await capture.settle();
        const [page] = context.pages();
        if (page !== undefined) {
          record.perf = await capture.readPagePerf(page);
        }
        return close(...args);
      };
      return context;
    });
  } finally {
    for (const record of opened) record.capture.beginTeardown();
    // Hand the records to `_autoPerfSpan`, which builds the spans. It cannot
    // read this fixture's value directly (it does not depend on it), and a
    // module-level handoff keyed by span id is the same shape `actions.ts`
    // already uses. Safe on ordering: auto fixtures set up first and so tear
    // down last, meaning this runs before `_autoPerfSpan`'s teardown reads it.
    tracedContextRecords.set(testSpanId, opened);
  }
};

export const autoPerfSpanFixture = [
  async (
    {
      page,
      testSpanId,
      _lifecycleStart,
    }: { page: Page; testSpanId: string; _lifecycleStart: number },
    use: Use<void>,
    testInfo: TestInfo,
  ) => {
    // `_lifecycleStart` was stamped before the browser context and page were
    // built; this fixture's body runs after both, so the gap between them is
    // the context-mint cost — otherwise invisible to the trace.
    const lifecycleStartMs = _lifecycleStart;
    const perfSpanEntryMs = Date.now();

    // Arm the one-boot-per-page budget (#867). This fixture is `auto`, and
    // Playwright sets auto fixtures up before requested ones, so arming
    // happens before `registeredPage` navigates — which is what lets the
    // budget see a test's very first document load rather than only later ones.
    trackBoots(page);

    // Capture attaches before the phase switch below, so any pre-test traffic
    // is measured rather than invisible (#794). Attribution is a separate
    // concern and still waits — fusing the two is what once left the (since
    // removed, #792) per-test warmup's duration measured nowhere.
    const capture = await attachTraceCapture(page.context());
    captureByTestSpanId.set(testSpanId, capture);

    const traceContext = traceContextFromEnvironment();
    // Records starting from here belong to the test. Switched at the same
    // moment as the traceparent so the two stay in lockstep.
    capture.setPhase("test");
    await applyTestTraceparent(
      page.context(),
      traceContext.traceId,
      testSpanId,
    );
    const testStartMs = Date.now();
    const testKey = `${testInfo.file}::${testInfo.title}::${testInfo.project.name}::${testInfo.retry}`;
    const { requests, navigations } = capture.sinkFor("test");

    setCurrentActionTestKey(testKey);
    let budgetFailures: string[] = [];
    try {
      await use();
    } finally {
      // The exported test span ends before teardown work begins. New browser
      // diagnostics are deliberately sinkless from this point onward.
      capture.beginTeardown();
      setCurrentActionTestKey(null);
      // Collect-and-clear unconditionally, so a test that failed cannot leak
      // its budget state into the next test in this worker. Whether to FAIL on
      // it is decided at the very end of this fixture, which a failing test
      // never reaches — a budget failure must never mask the real error.
      budgetFailures = takeBudgetFailures();
    }

    const endMs = Date.now();
    const actions = drainActionsForTest(testKey);
    // Per-document harvests are fired from `load` handlers, so they can still
    // be in flight here. Awaiting them is what makes the boot decomposition
    // available for EVERY navigation rather than just whichever ones happened
    // to land before teardown.
    await capture.settle();
    const pagePerfSummary: PagePerfSummary = await capture.readPagePerf(page);

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
    const navigationSummary = navigationSummariesFrom(
      navigations,
      requests,
      actions,
      (navigationId) => capture.timingFor(navigationId),
    );
    const navigationTelemetry = navigationTopTelemetryFrom(
      navigationSummary,
      navigations.length,
    );
    const topNavigations = navigationTelemetry.topNavigations;

    const browserDiagnosticProjection = browserDiagnosticSpanProjectionFor(
      "e2e.test",
      capture,
    );

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
      // Every capped list reports what it dropped, so truncation is never
      // silent as the suite grows. Raising the caps would only move the cliff,
      // and OTLP attribute size limits are real (#794).
      otlpAttribute(
        "e2e.request_top_slow_dropped",
        requests.length - topSlowRequests.length,
      ),
      ...browserDiagnosticProjection.attributes,
      otlpAttribute(
        "e2e.navigation_json",
        JSON.stringify(pagePerfSummary.navigation),
      ),
      otlpAttribute(
        "e2e.resource_summary_json",
        JSON.stringify(pagePerfSummary.resources),
      ),
      // The two counts below are the genuinely lossy ones: unlike actions,
      // requests and navigations, nothing else on the span records how many
      // resource or long-task entries were discarded. `long_tasks_json` is
      // worse still — a TAIL slice, so it is the EARLIEST long tasks that go.
      otlpAttribute(
        "e2e.resource_top_slow_dropped",
        pagePerfSummary.resources.droppedCount,
      ),
      otlpAttribute(
        "e2e.long_tasks_json",
        JSON.stringify(pagePerfSummary.longTasks),
      ),
      otlpAttribute(
        "e2e.long_tasks_dropped",
        pagePerfSummary.longTasksDroppedCount,
      ),
      otlpAttribute("e2e.action_count", actions.length),
      otlpAttribute("e2e.action_top_json", JSON.stringify(topActions)),
      otlpAttribute(
        "e2e.action_top_dropped",
        actions.length - topActions.length,
      ),
      otlpAttribute("e2e.navigation_count", navigations.length),
      otlpAttribute("e2e.navigation_top_json", navigationTelemetry.json),
      otlpAttribute("e2e.navigation_top_dropped", navigationTelemetry.dropped),
      // Every `jaunder.*` mark the CSR client emitted, per navigation, keyed by
      // navigation id. Discovered by prefix — the names live only in Rust, so a
      // new mark appears here with no change to this file.
      otlpAttribute(
        "e2e.boot_marks_json",
        JSON.stringify(
          navigations.map((navigation) => ({
            id: navigation.id,
            marks: capture.timingFor(navigation.id)?.marks ?? [],
          })),
        ),
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
          otlpAttribute("navigation.request_failed", navigation.requestFailed),
        ].filter(
          (attribute): attribute is NonNullable<typeof attribute> =>
            attribute !== null,
        ),
      ),
    );

    // The lifecycle envelope. `e2e.test` keeps its own span id, its start/end
    // range and its whole attribute set — widening it would have been the
    // smaller change and would have silently redefined "in-span time", making
    // every number the #788 investigation published non-comparable. Instead the
    // previously-invisible phases become properly-contained sibling spans, so
    // interval-union analysis works on them unchanged (#794).
    //
    // Reparenting `e2e.test` is safe by construction: the analyzer selects on
    // the exact span name (`s.name == "e2e.test"`, so `e2e.test.lifecycle`
    // cannot collide) and the flow-coverage extractor walks parent_span_id
    // UPWARD to an `e2e.test`-named span, so an extra ancestor above it changes
    // nothing (#681).
    const lifecycleSpanId = newSpanId();
    const exportStartMs = Date.now();

    // Identity attributes every span in the tree carries. `e2e.project` is not
    // decoration: `traces analyze --project <name>` drops any `e2e.`-prefixed
    // span whose `e2e.project` differs, so an unstamped span reads as "wrong
    // project" and the whole tree vanishes under that filter.
    const identity = () =>
      [
        otlpAttribute("e2e.file", testInfo.file),
        otlpAttribute("e2e.test", testInfo.title),
        otlpAttribute("e2e.project", testInfo.project.name),
      ].filter(
        (attribute): attribute is NonNullable<typeof attribute> =>
          attribute !== null,
      );

    const phaseSpan = (
      name: string,
      startMs: number,
      phaseEndMs: number,
      extra: ReturnType<typeof identity> = [],
    ) =>
      buildSpan({
        traceContext,
        name,
        parentSpanId: lifecycleSpanId,
        kind: "client",
        startMs,
        endMs: phaseEndMs,
        attributes: [...identity(), ...extra],
      });

    const spans = [
      buildSpan({
        traceContext,
        name: "e2e.test.lifecycle",
        spanId: lifecycleSpanId,
        kind: "client",
        startMs: lifecycleStartMs,
        endMs: exportStartMs,
        attributes: [
          ...identity(),
          otlpAttribute("e2e.status", testInfo.status),
          otlpAttribute("e2e.retry", testInfo.retry),
          otlpAttribute("e2e.total_ms", exportStartMs - lifecycleStartMs),
        ].filter(
          (attribute): attribute is NonNullable<typeof attribute> =>
            attribute !== null,
        ),
      }),
      phaseSpan("e2e.context_mint", lifecycleStartMs, perfSpanEntryMs),
      buildSpan({
        traceContext,
        name: "e2e.test",
        // The id the server already saw as the inbound traceparent's
        // parent-span-id, so the span this test exports is the one its request
        // spans point at (#681).
        spanId: testSpanId,
        parentSpanId: lifecycleSpanId,
        kind: "client",
        startMs: testStartMs,
        endMs,
        attributes,
        events: [...requestEvents, ...actionEvents, ...navigationEvents],
      }),
      phaseSpan("e2e.teardown", endMs, exportStartMs),
    ];

    // One span per extra context the spec opened via `tracedContext`. Without
    // these a multi-context test under-reports its own client cost — the
    // Private-post visibility test drives 9 `page.goto`s but reported
    // navigation_count 3, because only the default page was instrumented.
    for (const record of tracedContextRecords.get(testSpanId) ?? []) {
      await record.capture.settle();
      const sink = record.capture.sinkFor("test");
      const pageNavigationTelemetry = navigationTopTelemetryFrom(
        navigationSummariesFrom(
          sink.navigations,
          sink.requests,
          actions,
          (navigationId) => record.capture.timingFor(navigationId),
        ),
        sink.navigations.length,
      );
      const pageBrowserDiagnosticProjection =
        browserDiagnosticSpanProjectionFor("e2e.page", record.capture);
      spans.push(
        phaseSpan(
          "e2e.page",
          testStartMs,
          endMs,
          [
            otlpAttribute("e2e.request_count", sink.requests.length),
            otlpAttribute("e2e.navigation_count", sink.navigations.length),
            otlpAttribute(
              "e2e.navigation_top_json",
              pageNavigationTelemetry.json,
            ),
            otlpAttribute(
              "e2e.navigation_top_dropped",
              pageNavigationTelemetry.dropped,
            ),
            otlpAttribute(
              "e2e.resource_summary_json",
              JSON.stringify(record.perf?.resources ?? null),
            ),
            otlpAttribute(
              "e2e.navigation_json",
              JSON.stringify(record.perf?.navigation ?? null),
            ),
            ...pageBrowserDiagnosticProjection.attributes,
          ].filter(
            (attribute): attribute is NonNullable<typeof attribute> =>
              attribute !== null,
          ),
        ),
      );
    }
    tracedContextRecords.delete(testSpanId);
    captureByTestSpanId.delete(testSpanId);

    try {
      await exportSpans(spans);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.warn(`[e2e-otel] test export failed: ${message}`);
    }

    // Last, so the trace above is exported either way. Only reached when the
    // test body passed, so this can never mask a real failure.
    if (budgetFailures.length > 0) {
      throw new Error(
        `the per-page document-load budget failed (#867):\n` +
          budgetFailures.map((line) => `  - ${line}`).join("\n") +
          `\nAn undeclared second load either belongs in the app (move within ` +
          `it with navigateInApp) or is deliberate, and then it is declared ` +
          `with allowSecondBoot(page, "<reason>") — or, if whether the load ` +
          `happens at all depends on the browser engine, with ` +
          `allowEngineDependentBoot(page, "<reason>"). An allowance that nothing ` +
          `consumed does not expire: it waits and silently absorbs the next ` +
          `extra document load, disarming the budget. Either the load it ` +
          `authorised no longer happens — delete the declaration — or it moved, ` +
          `and the declaration should move with it.`,
      );
    }
  },
  { auto: true },
] satisfies AutoFixture<
  {
    page: Page;
    testSpanId: string;
    _lifecycleStart: number;
  },
  void
>;

/**
 * Auto-applied Playwright fixture (`_autoPerfSpan`, `auto: true`) that wraps every
 * test in OTel capture: it instruments page requests, navigations, and the CSR mount,
 * folds in the action records from actions.ts, and emits a single `e2e.test` span
 * on teardown.
 *
 * Timeout scaling is ambient (#261): a second auto fixture (`_autoTestTimeout`)
 * gives every test a scaled `DEFAULT_TEST_BUDGET_MS` whole-test budget, so tests
 * no longer name `testInfo` or a raw budget just to set a timeout. That budget
 * covers every test in the suite (#270) — reaching for `setTestBudget(ms)` is not
 * the default move. The two tests that do call it poll for eventually-consistent
 * state — one the feed cache, one the WebSub hub-ping capture — on deadlines that
 * exceed the ambient budget; both derive their budget from those deadlines rather
 * than restating a number. The `firstNav` fixture
 * value and `registeredPage` fixture supply the scaled cold-WASM first-nav budget
 * and a pre-registered page. This module also exports the underlying
 * slow-browser / worker-contention scalers (`slowBrowser*`), which remain the way
 * to size an individual assertion or navigation.
 */

import {
  expect,
  test as base,
  type Browser,
  type BrowserContext,
  type Page,
  type TestInfo,
} from "@playwright/test";
import {
  drainActionsForTest,
  setCurrentActionTestKey,
  type ActionRecord,
} from "./actions";
import {
  attachTraceCapture,
  type BootMark,
  type NavigationRecord,
  type PagePerfSummary,
  type RequestRecord,
  type TraceCapture,
} from "./capture-trace";
import {
  buildSpan,
  exportSpans,
  makeEvent,
  newSpanId,
  otlpAttribute,
  traceContextFromEnvironment,
} from "./otel";
import {
  BASE_URL,
  generateUsername,
  goto,
  setAndVerifyEmail,
  TEST_PASSWORD,
} from "./helpers";
import { readEmailLines, type CapturedEmail } from "./mail";
import { MOUNTED_ATTR, MOUNTED_SELECTOR } from "./mount";
import { pollUntil } from "./polling";
import { applySeededSession, seedUserViaTool } from "./seed";

/** One extra context a spec opened via `tracedContext`, plus the client-side
 *  perf read from it just before it closed. `_autoPerfSpan` turns each into an
 *  `e2e.page` span. */
type TracedContextRecord = {
  capture: TraceCapture;
  perf: PagePerfSummary | null;
};

/** Handoff from the `tracedContext` fixture to `_autoPerfSpan`, keyed by the
 *  test's span id. Drained when the spans are built. */
const tracedContextRecords = new Map<string, TracedContextRecord[]>();

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
  /** Decomposition of `commitToMountMs`. All document-relative (see
   *  `capture-trace.ts`), so comparable to each other but not to the wall-clock
   *  fields above. `null` where the document did not report the input. */
  wasmFetchStartMs: number | null;
  wasmFetchMs: number | null;
  wasmInstantiateMs: number | null;
  bootPhases: Record<string, number> | null;
  /** Mount-ready → the last mount-path request finishing. Covers what
   *  `commitToMountMs` does NOT: `data-mounted` is set the instant
   *  `mount_to_body` returns, so the shell/route fetches resolve after it. */
  mountToSettledMs: number | null;
};

/**
 * Decompose one document's boot marks into consecutive phase durations.
 *
 * Ordered by observed `startTime` rather than by an expected name sequence, so a
 * mark added in Rust that this file has never heard of still lands in the right
 * place. Returns `null` when fewer than two marks were seen — one mark yields no
 * interval, and reporting `{}` would read as "measured, all zero".
 */
function bootPhasesFrom(marks: BootMark[]): Record<string, number> | null {
  if (marks.length < 2) return null;
  const sorted = [...marks].sort(
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
 * KNOWN APPROXIMATION: `requests` is the default context's sink, but `actions`
 * is the whole test's — including `flow.*` driven on `tracedContext` pages. An
 * action on an unrelated context can therefore close the boundary early and
 * truncate this window, biasing the figure DOWN. Actions are not tagged with the
 * context that ran them (`ActionRecord` carries a page URL, not an identity), so
 * closing this needs a wider change than #794; an under-estimate was preferred
 * to over-attributing a later navigation's fetches to this one.
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
// Default the warmup URL to BASE_URL (env-aware, #249) rather than a second
// hardcoded :3000; the explicit JAUNDER_E2E_WARMUP_URL override still wins below.
const defaultWarmupUrl = `${BASE_URL}/`;
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
    await page.waitForSelector(MOUNTED_SELECTOR, {
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

/** The ambient whole-test budget every test receives via the `_autoTestTimeout`
 *  auto fixture (scaled per browser / worker contention). Every test in the suite
 *  fits inside it; the two that call `setTestBudget(ms)` do so because their own
 *  polling deadlines exceed it, not because they are merely slow (#270).
 *
 *  Sized against the suite's measured worst case: at `workers=2` this scales to
 *  45s chromium / 66s elsewhere, against a measured worst of 24.2s / 34.6s over
 *  19 green CI runs x 4 combos. The 18 per-test budgets deleted in #270 were
 *  validated at `workers>=2`. At `workers=1` the contention scale is 1.0, so
 *  chromium gets 30s here — and `cargo xtask e2e-local` defaults to 1 worker
 *  against the slower debug wasm build. The sharpest case is
 *  `visibility.spec.ts`'s "Public post is visible to anonymous" test: it polls a
 *  hard 25s deadline of its own, so at 30s it has only ~5s left for setup, and a
 *  whole-test timeout there is that collision rather than a slow test. If the
 *  heaviest specs (visibility, audiences) start timing out there, run with
 *  `JAUNDER_E2E_WORKERS=2`, or re-add a deliberate budget derived from whatever
 *  deadline the test actually needs. Do not raise this constant to fix it —
 *  `test.slow()` triples it, so raising it inflates the suite's largest budgets
 *  3:1. */
export const DEFAULT_TEST_BUDGET_MS = 30_000;

/** Raise the current test's whole-test budget to a scaled `chromiumBudgetMs`.
 *  Call as the FIRST line of a test body, before any awaited setup, so that
 *  setup runs under the raised budget rather than the ambient default. Reads
 *  `test.info()` internally, so the call site names neither `testInfo` nor the
 *  scaler. Only tests whose budget exceeds `DEFAULT_TEST_BUDGET_MS` need it. */
export function setTestBudget(chromiumBudgetMs: number): void {
  const info = test.info();
  info.setTimeout(slowBrowserTimeoutMs(info, chromiumBudgetMs));
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

/**
 * Mints a browser context already pointed at this test's `e2e.test` span.
 *
 * **Use this instead of `browser.newContext()` in specs.** A raw context does not
 * inherit the config-level `extraHTTPHeaders`, so its traffic carries the run-wide
 * traceparent from `playwright.config.ts` — indistinguishable across a
 * `fullyParallel` suite, so every server-fn hit it drives lands in the coverage
 * gate's orphan bucket instead of being attributed to this test (#681). Calling
 * `applyTestTraceparent` by hand at each site works too, but it is exactly the
 * kind of step that gets forgotten; this closes over the ids so it cannot be.
 *
 * The caller still owns the context's lifetime (`close()` it as before).
 * Enforced by the `traced-context` static check.
 */
export type NewTracedContext = (
  options?: Parameters<Browser["newContext"]>[0],
) => Promise<BrowserContext>;

/** A uniquely-named account provisioned per test. `password` is the literal
 *  seeded-account password; `email` is the deterministic unique address this
 *  account uses when it sets/verifies email. The remaining fields are the
 *  seed record (#791): everything needed to apply this session to another
 *  context via `applySeededSession`. */
export type TestUser = {
  username: string;
  password: string;
  email: string;
  token: string;
  setCookie: string;
  marker: string;
  markerKey: string;
  isOperator: boolean;
};

/** A recipient-scoped mail waiter bound to one `TestUser.email`. Each call
 *  returns that recipient's next unseen message (FIFO), so parallel tests
 *  never consume each other's mail. */
export type Mailbox = {
  waitForNewEmail(timeoutMs?: number): Promise<CapturedEmail>;
};

const test = base.extend<{
  _lifecycleStart: number;
  _autoTestTimeout: void;
  _autoPerfSpan: void;
  testSpanId: string;
  tracedContext: NewTracedContext;
  firstNav: number;
  registeredPage: Page;
  user: TestUser;
  mailbox: Mailbox;
  verifiedUser: TestUser;
}>({
  // The id of this test's `e2e.test` span, minted BEFORE the test body so it can
  // be propagated as the traceparent parent-span-id on every browser context the
  // test uses. Server request spans then carry it as `parentSpanId`, which is how
  // the flow-coverage gate attributes a server-fn hit to the test that caused it
  // (#681).
  //
  // It has to be a fixture, not a local in `_autoPerfSpan`: `user` and
  // `verifiedUser` build their own throwaway contexts and are independent
  // fixtures, so they cannot read a value minted inside another one.
  // The earliest moment this test can observe. Everything before `_autoPerfSpan`
  // — browser context mint, page creation, the rest of fixture setup — happens
  // after this stamp and before that fixture's body, which is the only way to
  // size it: `_autoPerfSpan` declares `{ page }`, so Playwright has already built
  // the context and page by the time its own body runs (#794).
  //
  // ORDERING IS LOAD-BEARING AND FRAGILE. Playwright sets auto fixtures up in
  // *registration* order (an insertion-ordered Map, stable-sorted worker-before-
  // test), so this works only because this key precedes `_autoTestTimeout` and
  // `_autoPerfSpan` in this object literal, and because it depends on nothing.
  // Reorder the keys and `e2e.context_mint` silently collapses to zero width —
  // which is exactly what its non-zero-duration check is there to catch.
  _lifecycleStart: [
    async ({}, use) => {
      await use(Date.now());
    },
    { auto: true },
  ],

  testSpanId: async ({}, use) => {
    await use(newSpanId());
  },

  // The sanctioned way for a spec to open an extra browser context — see
  // `NewTracedContext`. Closes over this test's trace ids so the traceparent
  // cannot be omitted at the call site.
  tracedContext: async ({ browser, testSpanId }, use) => {
    const { traceId } = traceContextFromEnvironment();
    const opened: TracedContextRecord[] = [];
    await use(async (options) => {
      const context = await browser.newContext(options);
      await applyTestTraceparent(context, traceId, testSpanId);
      // Same instrumentation the default page gets, through the same code path —
      // extra contexts used to have none, so a multi-context test under-reported
      // its own client cost (#794).
      const capture = await attachTraceCapture(context);
      capture.setPhase("test");
      const record: TracedContextRecord = { capture, perf: null };
      opened.push(record);

      // Snapshot the client-side perf BEFORE the context closes. `on("close")`
      // fires *after* closing, when `page.evaluate` would throw — and the caller
      // owns this context's lifetime, so wrapping `close` is the only hook that
      // reliably runs while a page is still alive.
      const close = context.close.bind(context);
      context.close = async (...args: Parameters<typeof close>) => {
        const [page] = context.pages();
        if (page !== undefined) {
          record.perf = await capture.readPagePerf(page);
        }
        return close(...args);
      };
      return context;
    });
    // Hand the records to `_autoPerfSpan`, which builds the spans. It cannot
    // read this fixture's value directly (it does not depend on it), and a
    // module-level handoff keyed by span id is the same shape `actions.ts`
    // already uses. Safe on ordering: auto fixtures set up first and so tear
    // down last, meaning this runs before `_autoPerfSpan`'s teardown reads it.
    tracedContextRecords.set(testSpanId, opened);
  },

  // Ambient whole-test timeout. `auto`, so it applies to EVERY test; Playwright
  // sets up auto fixtures before any requested fixture, so this budget is in
  // force before `user`/`verifiedUser`/`registeredPage` setup runs (covering the
  // out-of-band flows that used to hand-roll their own `setTimeout`). The default
  // covers every test; the rare test whose own deadlines exceed it derives a
  // budget with `setTestBudget(ms)` (#270).
  _autoTestTimeout: [
    async ({}, use, testInfo) => {
      testInfo.setTimeout(
        slowBrowserTimeoutMs(testInfo, DEFAULT_TEST_BUDGET_MS),
      );
      await use();
    },
    { auto: true },
  ],

  // The scaled first-navigation (cold-WASM) budget for the modal 10_000 sites,
  // so tests consume `{ firstNav }` instead of recomputing it. Sites that need a
  // larger first-nav budget keep an explicit
  // `slowBrowserFirstNavigationTimeoutMs(testInfo, N)` call.
  firstNav: async ({}, use, testInfo) => {
    await use(slowBrowserFirstNavigationTimeoutMs(testInfo, 10_000));
  },

  // The test's own `page`, already signed in with a fresh unique seeded
  // account and mounted at `/` — collapsing the old
  // `register(page, firstNav)` preamble. Seeds the DEFAULT page's context (not
  // a new one) so it stays instrumented by `_autoPerfSpan`, and still yields a
  // mounted page because its consumers assume one (spec D8). For tests that
  // discard the username; tests that need the username/credentials use
  // `signInAsNewUser(...)` directly or the `user`/`verifiedUser` fixtures.
  registeredPage: async ({ page, firstNav }, use) => {
    const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
    await applySeededSession(page.context(), record);
    await goto(page, "/", { timeout: firstNav });
    await use(page);
  },
  // A uniquely-named account, seeded out-of-band with no browser involvement
  // at all — no throwaway context, no page, no navigation. Lazy: only
  // provisioned for tests that destructure `user`.
  user: async ({}, use) => {
    const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
    await use({
      username: record.username,
      password: TEST_PASSWORD,
      email: `${record.username}@example.com`,
      token: record.token,
      setCookie: record.setCookie,
      marker: record.marker,
      markerKey: record.markerKey,
      isOperator: record.isOperator,
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
    const waitForNewEmail = async (timeoutMs = 5_000): Promise<CapturedEmail> =>
      pollUntil(
        "wait.mail",
        () => {
          const mails = matching();
          if (mails.length <= cursor) return undefined;
          const next = mails[cursor];
          cursor += 1;
          return next;
        },
        {
          intervalMs: 100,
          timeoutMs,
          describe: `an email to ${user.email}`,
        },
      );
    await use({ waitForNewEmail });
  },

  // `user` plus the email set-and-verify flow, driven through `mailbox`, all
  // out-of-band so the test's `page` stays logged out. The session is applied
  // to the throwaway context from the seed record (no UI login, #791); the
  // set-email/verify flow itself still goes through the UI — it is
  // `email::request_verification` / `email::verify` coverage. Yields the same
  // credentials; the account now has a verified email.
  verifiedUser: async ({ tracedContext, user, mailbox }, use) => {
    // The out-of-band setup below (newContext + seeded session + set-email +
    // verify) runs before the test body; the ambient `_autoTestTimeout` auto
    // fixture (which runs before this one) has already scaled the whole-test
    // budget, so this setup is covered without a hand-rolled `setTimeout` here.
    const context = await tracedContext();
    const page = await context.newPage();
    await applySeededSession(context, user);
    await setAndVerifyEmail(page, user.email, mailbox);
    await context.close();
    await use(user);
  },

  _autoPerfSpan: [
    async ({ page, testSpanId, _lifecycleStart }, use, testInfo) => {
      // `_lifecycleStart` was stamped before the browser context and page were
      // built; this fixture's body runs after both, so the gap between them is
      // the context-mint cost that used to be invisible.
      const lifecycleStartMs = _lifecycleStart;
      const perfSpanEntryMs = Date.now();

      // Capture attaches BEFORE the warmup, so the warmup's own traffic is
      // measured rather than invisible. Attribution is a separate concern and
      // still waits (see below) — fusing the two is what left the warmup's
      // duration measured nowhere (#794).
      const capture = await attachTraceCapture(page.context());
      const warmupStartMs = Date.now();
      await warmupPageContext(page, testInfo);
      // `null` when warmup is off, so no `e2e.warmup` span is emitted at all
      // rather than a misleading zero-width one.
      const warmupEndMs = parseBooleanFlag(process.env.JAUNDER_E2E_WARMUP)
        ? Date.now()
        : null;

      const traceContext = traceContextFromEnvironment();
      // Records starting from here belong to the test, not the warmup. Switched
      // at the same moment as the traceparent so the two stay in lockstep.
      capture.setPhase("test");
      // The test's own context, so its requests are attributable too. Applied
      // after the warmup so warmup traffic (which is not part of the test) stays
      // out of the attribution (#681).
      await applyTestTraceparent(
        page.context(),
        traceContext.traceId,
        testSpanId,
      );
      const testStartMs = Date.now();
      const testKey = `${testInfo.file}::${testInfo.title}::${testInfo.project.name}::${testInfo.retry}`;
      const { requests, navigations } = capture.sinkFor("test");

      setCurrentActionTestKey(testKey);
      try {
        await use();
      } finally {
        setCurrentActionTestKey(null);
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
      const navigationSummary: NavigationSummary[] = navigations
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
          const timing = capture.timingFor(navigation.id);
          const wasm = timing?.wasm ?? null;
          const bootEntry = timing?.marks.find((mark) =>
            mark.name.endsWith(".boot.entry"),
          );
          // Positional, not `startedMs >`: navigations are pushed in start order,
          // and two can share a `Date.now()` millisecond. A `>` search skips the
          // tied neighbour and lands on the one after it, widening the window so
          // a later navigation's fetches get counted as this one's settle.
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
            wasmFetchMs: wasm?.durationMs ?? null,
            // Rust cannot see its own fetch or instantiation — it only starts
            // running at `boot.entry` — so this span is derived, not marked.
            wasmInstantiateMs:
              wasm !== null && bootEntry !== undefined
                ? bootEntry.startTime - wasm.responseEndMs
                : null,
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
        // Every capped list reports what it dropped, so truncation is never
        // silent as the suite grows. Raising the caps would only move the cliff,
        // and OTLP attribute size limits are real (#794).
        otlpAttribute(
          "e2e.request_top_slow_dropped",
          requests.length - topSlowRequests.length,
        ),
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
        otlpAttribute(
          "e2e.navigation_top_json",
          JSON.stringify(topNavigations),
        ),
        otlpAttribute(
          "e2e.navigation_top_dropped",
          navigations.length - topNavigations.length,
        ),
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

      const warmupSink = capture.sinkFor("warmup");
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

      if (warmupEndMs !== null) {
        // Warmup TRAFFIC stays unattributed by design (#681's orphan bucket);
        // what was missing was its DURATION, which is measured nowhere today.
        // These counts are the warmup's own, kept out of `e2e.test`'s.
        spans.push(
          phaseSpan(
            "e2e.warmup",
            warmupStartMs,
            warmupEndMs,
            [
              otlpAttribute("e2e.request_count", warmupSink.requests.length),
              otlpAttribute(
                "e2e.navigation_count",
                warmupSink.navigations.length,
              ),
            ].filter(
              (attribute): attribute is NonNullable<typeof attribute> =>
                attribute !== null,
            ),
          ),
        );
      }

      // One span per extra context the spec opened via `tracedContext`. Without
      // these a multi-context test under-reports its own client cost — the
      // Private-post visibility test drives 9 `page.goto`s but reported
      // navigation_count 3, because only the default page was instrumented.
      for (const record of tracedContextRecords.get(testSpanId) ?? []) {
        const sink = record.capture.sinkFor("test");
        spans.push(
          phaseSpan(
            "e2e.page",
            testStartMs,
            endMs,
            [
              otlpAttribute("e2e.request_count", sink.requests.length),
              otlpAttribute("e2e.navigation_count", sink.navigations.length),
              otlpAttribute(
                "e2e.resource_summary_json",
                JSON.stringify(record.perf?.resources ?? null),
              ),
              otlpAttribute(
                "e2e.navigation_json",
                JSON.stringify(record.perf?.navigation ?? null),
              ),
            ].filter(
              (attribute): attribute is NonNullable<typeof attribute> =>
                attribute !== null,
            ),
          ),
        );
      }
      tracedContextRecords.delete(testSpanId);

      try {
        await exportSpans(spans);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        console.warn(`[e2e-otel] test export failed: ${message}`);
      }
    },
    { auto: true },
  ],
});

export { expect, test };

/**
 * The per-`Page` document-load budget (#867).
 *
 * The suite tests a pure-CSR SPA, so a page pays a document load once — on
 * entry — and moves within the app thereafter. This module counts the loads and
 * fails a page that boots twice without saying why. It is the test-side
 * counterpart of the app-side `no-full-reload` rule (ADR-0076).
 *
 * ## Why `domcontentloaded` and not `framenavigated`
 *
 * `framenavigated` also fires for same-document `history.pushState` navigation,
 * which is exactly what an in-app router move *is* — counting it would flag
 * every conversion this work makes. `domcontentloaded` fires only when a real
 * document is parsed, so a router push is invisible to it and a full load never
 * is. `bootBudget.spec.ts` pins both halves of that claim.
 *
 * ## Why the page, not the wrapper
 *
 * Counting inside `goto` would leave every raw `page.goto` as a blind spot,
 * including the sites that legitimately cannot use the wrapper (the CLS probe
 * holds the wasm so mount never completes, so `goto`'s `waitForMount` would
 * hang). Subscribing to the page's own event sees every document load however
 * it was issued.
 *
 * ## How a violation surfaces
 *
 * The event handler cannot reject its caller's promise, so it records the
 * violation and `throwIfViolated` — called from `goto` — raises it. A violation
 * caused by a raw `page.goto` therefore surfaces on the next budget-aware call
 * rather than at the offending line; the message names both URLs so the page is
 * still identifiable.
 */

import type { Page } from "@playwright/test";

type BudgetState = {
  /** Document loads on this page, in order, as URLs. */
  loads: string[];
  /** Unconsumed `allowSecondBoot` reasons, consumed in call order. */
  allowances: string[];
  /** The reasons consumed so far — the page's derived exemption census. */
  consumed: string[];
  /** Set on the first undeclared extra load; raised by `throwIfViolated`. */
  violation?: string;
};

const states = new WeakMap<Page, BudgetState>();

/**
 * The pages armed since the last {@link takeOrphanedAllowances} call, so that
 * teardown can sweep every page a test touched rather than only its default one.
 *
 * A strong `Set` where the state map is a `WeakMap`, deliberately: it is emptied
 * once per test, so it retains a page for at most one test rather than for the
 * run. Playwright runs a worker's tests serially in one process, so there is
 * exactly one test's worth of pages in here at a time.
 */
const tracked = new Set<Page>();

/**
 * A page's blank starting document is not a boot. Playwright opens every page
 * at `about:blank`, and whether that fires `domcontentloaded` is an engine
 * detail — counting it would make every real entry look like a second load.
 */
function isRealDocument(url: string): boolean {
  return url !== "about:blank" && url !== "";
}

/**
 * Arm the budget on `page`. Idempotent: `tracedContext` arms every page it
 * creates, so an explicit call in a test is a no-op rather than a double count.
 */
export function trackBoots(page: Page): void {
  if (states.has(page)) {
    tracked.add(page);
    return;
  }
  const state: BudgetState = { loads: [], allowances: [], consumed: [] };
  states.set(page, state);
  tracked.add(page);

  page.on("domcontentloaded", () => {
    const url = page.url();
    if (!isRealDocument(url)) return;
    state.loads.push(url);
    if (state.loads.length === 1) return;

    const allowance = state.allowances.shift();
    if (allowance !== undefined) {
      state.consumed.push(allowance);
      return;
    }
    state.violation ??=
      `second document load on this page: it booted at ${state.loads[0]}, ` +
      `then loaded ${url}. A page boots once (#867) — move within the app ` +
      `with navigateInApp, or, if this page's cold render is the subject, ` +
      `declare it with allowSecondBoot(page, "<reason>").`;
  });
}

/**
 * Authorise one further document load on `page`, for a stated reason.
 *
 * One allowance covers one load, so a page that legitimately boots three times
 * calls this twice. The reason is required and is the record of what was
 * deliberately left alone — it is read by humans, never by the gate.
 */
export function allowSecondBoot(page: Page, reason: string): void {
  if (reason.trim() === "") {
    throw new Error(
      "allowSecondBoot needs a non-empty reason: it is the record of why this " +
        "page boots more than once (#867).",
    );
  }
  let state = states.get(page);
  if (state === undefined) {
    // Arming late. A declaration can only ever follow the page's entry load —
    // you cannot declare a *second* boot before the first — so the entry is
    // counted here rather than refused. Refusing instead would make a
    // declaration unusable on any page the fixtures had not already armed,
    // which is a deadlock: declarations are written before arming becomes
    // automatic, and arming cannot become automatic until they are written.
    trackBoots(page);
    state = states.get(page);
    state?.loads.push(page.url());
  }
  state?.allowances.push(reason);
}

/** Document loads counted on `page` so far. Zero if never armed. */
export function bootCount(page: Page): number {
  return states.get(page)?.loads.length ?? 0;
}

/** The reasons consumed on `page` — its derived exemption census. */
export function consumedReasons(page: Page): string[] {
  return [...(states.get(page)?.consumed ?? [])];
}

/** Allowances declared on `page` that no load has consumed yet. */
export function pendingReasons(page: Page): string[] {
  return [...(states.get(page)?.allowances ?? [])];
}

/**
 * Take the unconsumed allowances across every page armed since the last call,
 * clearing them and the tracked-page set.
 *
 * ## Why an unconsumed allowance is a defect, not slack
 *
 * An allowance does not expire. A declaration that authorises a load which never
 * happens sits in the queue and silently absorbs the *next* extra load — which
 * is precisely the undeclared second load the budget exists to catch. So an
 * over-declaration does not merely waste a line; it disarms the check for the
 * rest of the page's life, and does so invisibly. This is ADR-0094's
 * orphan-marker rule ("a marker whose site no longer exists fails") in runtime
 * form: an exemption nothing re-verifies must at least be checked to still apply.
 *
 * Returns one `"<entry url>: <reason>"` line per orphan. Always clears, so a
 * failing test cannot leak its allowances into the next test in the worker; the
 * caller decides whether to fail on the result.
 */
export function takeOrphanedAllowances(): string[] {
  const orphans: string[] = [];
  for (const page of tracked) {
    const state = states.get(page);
    if (state === undefined || state.allowances.length === 0) continue;
    const where = state.loads[0] ?? "(a page that never loaded)";
    for (const reason of state.allowances) {
      orphans.push(`${where}: ${reason}`);
    }
    state.allowances.length = 0;
  }
  tracked.clear();
  return orphans;
}

/**
 * Raise any recorded violation. Called by `goto` so an undeclared second load
 * fails the test rather than passing silently. The violation is cleared as it
 * is raised, so one budget breach produces one error.
 */
export function throwIfViolated(page: Page): void {
  const state = states.get(page);
  if (state?.violation === undefined) return;
  const { violation } = state;
  state.violation = undefined;
  throw new Error(violation);
}

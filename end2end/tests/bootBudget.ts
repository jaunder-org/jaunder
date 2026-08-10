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
  if (states.has(page)) return;
  const state: BudgetState = { loads: [], allowances: [], consumed: [] };
  states.set(page, state);

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
  const state = states.get(page);
  if (state === undefined) {
    throw new Error(
      "allowSecondBoot called on a page whose budget was never armed — call " +
        "trackBoots(page) first (fixtures arm it automatically).",
    );
  }
  state.allowances.push(reason);
}

/** Document loads counted on `page` so far. Zero if never armed. */
export function bootCount(page: Page): number {
  return states.get(page)?.loads.length ?? 0;
}

/** The reasons consumed on `page` — its derived exemption census. */
export function consumedReasons(page: Page): string[] {
  return [...(states.get(page)?.consumed ?? [])];
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

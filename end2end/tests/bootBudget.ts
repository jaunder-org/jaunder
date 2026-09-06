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
 * The price is that a document replaced before it reaches `DOMContentLoaded` is
 * counted by some engines and not others — measured on the pre-paint `/`→`/app`
 * redirect, which firefox counts and chromium does not. Such a load is declared
 * with `allowEngineDependentBoot`, which is scoped to that load's path because it
 * is the one declaration that can survive unconsumed; every other load takes the
 * exact, unscoped `allowSecondBoot`.
 *
 * ## Why the page, not the wrapper
 *
 * Counting inside `goto` would leave every raw `page.goto` as a blind spot,
 * including the sites that legitimately cannot use the wrapper (the CLS probe
 * holds the wasm so mount never completes, so `goto`'s `waitForMount` would
 * hang). Subscribing to the page's own event sees a document load whoever issued
 * it — with the engine-dependent caveat above: what it sees is the event, not the
 * navigation, so a document replaced before `DOMContentLoaded` is invisible to it
 * on the engines that never fire it.
 *
 * ## How a violation surfaces
 *
 * The event handler cannot reject its caller's promise, so it records the
 * violation and something else raises it. Two raisers, and both are needed:
 * `throwIfViolated` — called from `goto` — fails the test at the next
 * budget-aware call, which is the earliest and most informative moment; and the
 * teardown sweep (`takeBudgetFailures`) catches the rest, because a page whose
 * test issues no later `goto` reaches no such call, and the raw sites that
 * legitimately cannot use the wrapper are exactly the ones on that path. With
 * only the first, a violation on those pages would be detected and then
 * discarded. A violation caused by a raw `page.goto` therefore surfaces later
 * than the offending line; the message names both URLs so the page is still
 * identifiable.
 */

import type { Page } from "@playwright/test";

type Use<T> = (value: T) => Promise<void>;
type AutoFixture<Args, T> = [
  (args: Args, use: Use<T>) => Promise<void>,
  { auto: true },
];

/**
 * One declared further load. An engine-dependent allowance carries the `path` of
 * the load it was written for and matches nothing else; an exact allowance has no
 * path and covers the next load, whatever it is.
 */
type Allowance = {
  reason: string;
  /** The pathname this allowance is scoped to, or `undefined` for the exact form. */
  path?: string;
};

/**
 * A page's blank starting document is not a boot. Playwright opens every page
 * at `about:blank`, and whether that fires `domcontentloaded` is an engine
 * detail — counting it would make every real entry look like a second load.
 */
function isRealDocument(url: string): boolean {
  return url !== "about:blank" && url !== "";
}

/**
 * The pathname of a load or of a declared path. Origins and query strings vary
 * per run, while the route is what an engine-dependent declaration describes.
 */
function pathOf(url: string): string {
  return new URL(url, "http://budget.invalid").pathname;
}

function validateReason(reason: string, by: string): void {
  if (reason.trim() === "") {
    throw new Error(
      `${by} needs a non-empty reason: it is the record of why this ` +
        "page boots more than once (#867).",
    );
  }
}

/**
 * The browser-independent state machine behind the per-page budget.
 *
 * This is intentionally the only accounting seam: the page adapter supplies
 * actual `DOMContentLoaded` URLs, while cheap tests drive the same transitions
 * directly without inventing a Page or event emitter.
 */
export class BootBudgetAccounting {
  /** Document loads on this page, in order, as URLs. */
  private readonly loads: string[] = [];
  /** Declared further loads not yet consumed. */
  private readonly allowances: Allowance[] = [];
  /** Set on the first undeclared extra load. */
  private violation?: string;

  recordDocumentLoad(url: string): void {
    if (!isRealDocument(url)) return;
    this.loads.push(url);
    if (this.loads.length === 1) return;

    if (this.takeAllowance(url) !== undefined) return;
    this.violation ??=
      `second document load on this page: it booted at ${this.loads[0]}, ` +
      `then loaded ${url}. A page boots once (#867) — move within the app ` +
      `with navigateInApp, or, if this page's cold render is the subject, ` +
      `declare it with allowSecondBoot(page, "<reason>").`;
  }

  allowSecondBoot(reason: string): void {
    validateReason(reason, "allowSecondBoot");
    this.allowances.push({ reason });
  }

  allowEngineDependentBoot(path: string, reason: string): void {
    validateReason(reason, "allowEngineDependentBoot");
    this.allowances.push({ reason, path: pathOf(path) });
  }

  bootCount(): number {
    return this.loads.length;
  }

  pendingReasons(): string[] {
    return this.allowances.map((allowance) => allowance.reason);
  }

  takeFailures(): string[] {
    const violations =
      this.violation === undefined
        ? []
        : [`undeclared second load — ${this.violation}`];
    this.violation = undefined;
    const where = this.loads[0] ?? "(a page that never loaded)";
    const orphans = this.allowances
      .filter((allowance) => allowance.path === undefined)
      .map((allowance) => `${where}: ${allowance.reason}`);
    this.allowances.length = 0;
    return [...violations, ...orphans];
  }

  throwIfViolated(): void {
    if (this.violation === undefined) return;
    const { violation } = this;
    this.violation = undefined;
    throw new Error(violation);
  }

  private takeAllowance(url: string): Allowance | undefined {
    const path = pathOf(url);
    const scoped = this.allowances.findIndex(
      (allowance) => allowance.path === path,
    );
    const index =
      scoped === -1
        ? this.allowances.findIndex((allowance) => allowance.path === undefined)
        : scoped;
    if (index === -1) return undefined;
    const [taken] = this.allowances.splice(index, 1);
    return taken;
  }
}

const states = new WeakMap<Page, BootBudgetAccounting>();

/**
 * The pages armed since the last {@link takeBudgetFailures} call, so teardown
 * can sweep every page a test touched rather than only its default one.
 */
const tracked = new Set<Page>();

/**
 * Arm the budget on `page`. Idempotent: `tracedContext` arms every page it
 * creates, so an explicit call in a test is a no-op rather than a double count.
 */
export function trackBoots(page: Page): void {
  if (states.has(page)) {
    tracked.add(page);
    return;
  }
  const state = new BootBudgetAccounting();
  states.set(page, state);
  tracked.add(page);

  page.on("domcontentloaded", () => {
    state.recordDocumentLoad(page.url());
  });
}

/**
 * Arm the default page before requested fixtures can navigate it.
 *
 * Kept separate from performance capture so the late-arming contract can
 * disable only this policy while retaining normal tracing and teardown.
 */
export const autoBootBudgetFixture = [
  async ({ page }: { page: Page }, use: Use<void>) => {
    trackBoots(page);
    await use();
  },
  { auto: true },
] satisfies AutoFixture<{ page: Page }, void>;

/**
 * Authorise one further document load on `page`, for a stated reason.
 *
 * One allowance covers one load, so a page that legitimately boots three times
 * calls this twice. The reason is required and is the record of what was
 * deliberately left alone — it is read by humans, never by the gate. The count is
 * exact: an allowance nothing consumes fails the test (see
 * {@link takeBudgetFailures}). Use {@link allowEngineDependentBoot} for the rare
 * load whose very existence depends on the browser engine.
 */
export function allowSecondBoot(page: Page, reason: string): void {
  declare(page, reason, undefined, "allowSecondBoot");
}

/**
 * Authorise **at most one** further document load of `path` on `page`, for a
 * stated reason, where whether that load happens depends on the browser engine.
 *
 * Exempt from the orphan rule, and only for that reason: whether the load happens
 * is not the test's choice. Measured case — the pre-paint `location.replace` off
 * `/`: chromium replaces the document during head parsing, so `/` never reaches
 * `DOMContentLoaded` and the budget counts one load; firefox does fire it, and the
 * budget counts two. No fixed count is right for that flow, which is why this form
 * exists.
 *
 * **`path` is not decoration — it is what bounds the exemption.** An unscoped
 * orphan-exempt allowance survives the load it was written for and is then handed
 * to whatever loads next, so a genuinely undeclared load passes silently: the one
 * thing the budget exists to catch. Scoped, it matches only its own pathname (see
 * {@link pathOf}) and is inert against anything else. Pass a path, not a URL — the
 * origin is a per-run ephemeral port.
 *
 * **This is not the default and must not become one.** `allowSecondBoot` keeps
 * exact-count semantics and its orphan rule, and that rule is the only thing a
 * machine can check about a written exemption. Reach for this form only when the
 * load's existence genuinely varies by engine, and say in the reason *why* it
 * varies — "engine-dependent" on its own records nothing a reader can check.
 */
export function allowEngineDependentBoot(
  page: Page,
  path: string,
  reason: string,
): void {
  declare(page, reason, path, "allowEngineDependentBoot");
}

/**
 * The shared body of the two declaration forms. `path` scopes the allowance (the
 * engine-dependent form) or is `undefined` (the exact form); `by` names the caller
 * in errors.
 */
function declare(
  page: Page,
  reason: string,
  path: string | undefined,
  by: string,
): void {
  validateReason(reason, by);
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
    // Only if the page really has an entry to infer. Pushing unconditionally
    // would record `about:blank` as the entry on a page that has not navigated
    // yet, and then the page's *first* real load would look like its second and
    // consume this allowance — leaving the genuine second load uncounted. That
    // is the budget silently disarming itself, which is the one failure it
    // exists to prevent.
    const url = page.url();
    if (isRealDocument(url)) state?.recordDocumentLoad(url);
  }
  if (path === undefined) state?.allowSecondBoot(reason);
  else state?.allowEngineDependentBoot(path, reason);
}

/** Document loads counted on `page` so far. Zero if never armed. */
export function bootCount(page: Page): number {
  return states.get(page)?.bootCount() ?? 0;
}

/** Reasons declared on `page` that no load has consumed yet, either form. */
export function pendingReasons(page: Page): string[] {
  return states.get(page)?.pendingReasons() ?? [];
}

/**
 * Take every budget failure across the pages armed since the last call, clearing
 * them and the tracked-page set. Two kinds, violations first:
 *
 * 1. **An undeclared second load** — recorded by the event handler, which cannot
 *    reject its caller's promise. `throwIfViolated` raises one at the next
 *    budget-aware call, but a page whose test issues no further `goto` has no
 *    such call — and the sites that legitimately cannot use the wrapper are
 *    exactly the ones on that path. Without this sweep the budget would detect
 *    those loads and then discard them, which is detection without enforcement.
 * 2. **An unconsumed allowance.** An allowance does not expire. A declaration
 *    that authorises a load which never happens sits in the queue and silently
 *    absorbs the *next* extra load — precisely the undeclared second load the
 *    budget exists to catch. So an over-declaration does not merely waste a
 *    line; it disarms the check for the rest of the page's life, and does so
 *    invisibly. This is ADR-0094's orphan-marker rule ("a marker whose site no
 *    longer exists fails") in runtime form: an exemption nothing re-verifies
 *    must at least be checked to still apply. An `allowEngineDependentBoot`
 *    declaration is deliberately excluded: whether its load happens is the
 *    engine's choice, not the test's, so an unconsumed one is no evidence that
 *    the test over-declared.
 *
 *    **It is not therefore harmless, and the exclusion has a price this module
 *    pays rather than solves.** An unconsumed scoped allowance still absorbs a
 *    later load of the same path, and it blinds this rule by one slot on that
 *    page: with exact declarations A and B alongside a scoped one, if B's load
 *    regresses away, A and B are both spent by the loads that remain, the scoped
 *    allowance survives exempt, and B's disappearance is never reported. Nothing
 *    here can close that while loads carry no identity of their own — the path
 *    scope narrows it to same-path loads, which is why the scope is mandatory and
 *    why this form stays rare.
 *
 * A violation line leads with `undeclared second load —`; an orphan line is
 * `"<entry url>: <reason>"`. Always clears, so a failing test cannot leak either
 * kind into the next test in the worker; the caller decides whether to fail on
 * the result.
 */
export function takeBudgetFailures(): string[] {
  const violations: string[] = [];
  const orphans: string[] = [];
  for (const page of tracked) {
    const state = states.get(page);
    if (state === undefined) continue;
    const failures = state.takeFailures();
    for (const failure of failures) {
      if (failure.startsWith("undeclared second load —"))
        violations.push(failure);
      else orphans.push(failure);
    }
  }
  tracked.clear();
  // Violations first: a load that happened outranks a declaration for one that
  // did not.
  return [...violations, ...orphans];
}

/**
 * Raise any recorded violation. Called by `goto` so an undeclared second load
 * fails the test at the earliest budget-aware moment rather than at teardown.
 * The violation is cleared as it is raised, so one budget breach produces one
 * error and the teardown sweep does not report it a second time.
 */
export function throwIfViolated(page: Page): void {
  states.get(page)?.throwIfViolated();
}

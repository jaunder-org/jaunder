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

type BudgetState = {
  /** Document loads on this page, in order, as URLs. */
  loads: string[];
  /** Declared further loads not yet consumed — see {@link takeAllowance}. */
  allowances: Allowance[];
  /** Set on the first undeclared extra load; raised at the next `goto` or, if
   *  there is none, by the teardown sweep. */
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
 * The pathname of a load or of a declared path.
 *
 * **Pathname, not the whole URL:** the origin is an ephemeral `host:port` chosen
 * per run (`JAUNDER_E2E_BASE_URL`), so an origin-bearing key could never be
 * written in a test; and the query string carries per-run salts and tokens, which
 * would make a declaration match on one run and not the next. The route is what a
 * declaration is actually about. The dummy base only supplies the parser with an
 * origin to discard — a declared path is relative by construction.
 */
function pathOf(url: string): string {
  return new URL(url, "http://budget.invalid").pathname;
}

/**
 * Consume one allowance for a load of `url`, **a matching scoped allowance
 * first**, then the first exact one.
 *
 * Scoped-first is what makes the two forms compose. Take the pre-paint redirect:
 * an exact declaration for `/app` (which always lands) plus an engine-dependent
 * one for `/`. On firefox the `/` load must spend the `/` declaration — spending
 * the exact one there would leave `/app` with nothing to consume and fail the
 * page. On chromium `/` never arrives, `/app` matches no scoped allowance, and it
 * spends the exact one, leaving the scoped declaration unconsumed and exempt.
 *
 * The exact form stays unscoped: it must be consumed, so an unconsumed one is
 * already reported and cannot silently absorb anything.
 */
function takeAllowance(state: BudgetState, url: string): Allowance | undefined {
  const path = pathOf(url);
  const scoped = state.allowances.findIndex((a) => a.path === path);
  const index =
    scoped === -1
      ? state.allowances.findIndex((a) => a.path === undefined)
      : scoped;
  if (index === -1) return undefined;
  const [taken] = state.allowances.splice(index, 1);
  return taken;
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
  const state: BudgetState = { loads: [], allowances: [] };
  states.set(page, state);
  tracked.add(page);

  page.on("domcontentloaded", () => {
    const url = page.url();
    if (!isRealDocument(url)) return;
    state.loads.push(url);
    if (state.loads.length === 1) return;

    if (takeAllowance(state, url) !== undefined) return;
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
  declare(page, reason, pathOf(path), "allowEngineDependentBoot");
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
  if (reason.trim() === "") {
    throw new Error(
      `${by} needs a non-empty reason: it is the record of why this ` +
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
    // Only if the page really has an entry to infer. Pushing unconditionally
    // would record `about:blank` as the entry on a page that has not navigated
    // yet, and then the page's *first* real load would look like its second and
    // consume this allowance — leaving the genuine second load uncounted. That
    // is the budget silently disarming itself, which is the one failure it
    // exists to prevent.
    const url = page.url();
    if (isRealDocument(url)) state?.loads.push(url);
  }
  state?.allowances.push({ reason, path });
}

/** Document loads counted on `page` so far. Zero if never armed. */
export function bootCount(page: Page): number {
  return states.get(page)?.loads.length ?? 0;
}

/** Reasons declared on `page` that no load has consumed yet, either form. */
export function pendingReasons(page: Page): string[] {
  return (states.get(page)?.allowances ?? []).map((a) => a.reason);
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
    if (state.violation !== undefined) {
      violations.push(`undeclared second load — ${state.violation}`);
      state.violation = undefined;
    }
    const where = state.loads[0] ?? "(a page that never loaded)";
    for (const allowance of state.allowances) {
      if (allowance.path !== undefined) continue; // engine-dependent: exempt
      orphans.push(`${where}: ${allowance.reason}`);
    }
    state.allowances.length = 0;
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
  const state = states.get(page);
  if (state?.violation === undefined) return;
  const { violation } = state;
  state.violation = undefined;
  throw new Error(violation);
}

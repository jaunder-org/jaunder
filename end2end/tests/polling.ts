/**
 * The suite's single polling primitive (#794, gap 6).
 *
 * One instrumented helper, not a per-site `deadline`/`while`/`sleep`/`throw`
 * loop: uninstrumented copies are invisible to the trace (the WebSub ping test
 * lost ~17.6 s per combo that way), and instrumenting copies leaves places to
 * forget. There is exactly one place a capture-wait can be written, and it is
 * timed.
 * That is also why no lint guards this — the primitive is the enforcement.
 *
 * Built on Playwright's `expect(...).toPass()` rather than a hand-rolled loop:
 * the retry/timeout semantics are maintained upstream, and for probes that call
 * a Playwright API each attempt also shows up as a step in the HTML report and
 * trace viewer. (Probes that read the capture files with plain node `fs` get
 * OTel attribution only — no Playwright API runs per attempt.)
 */

import { expect } from "@playwright/test";
import { withTimedAction } from "./actions";

export type PollOptions = {
  /** Delay between attempts. Per-call: the sites range 100–500 ms. */
  intervalMs: number;
  /** Total budget. Per-call: the sites range 5 s–30 s. A single shared default
   *  would be a flake generator at the short end and a slow failure at the long. */
  timeoutMs: number;
  /** Names what was awaited, for the timeout message. */
  describe: string;
};

/**
 * Poll `probe` until it returns something other than `undefined`, and return
 * that value. Throws once `timeoutMs` elapses.
 *
 * `name` is the timed-action name the wait is recorded under (`wait.mail`,
 * `wait.websub_ping`, `wait.feed`), so the wait is attributed in the trace.
 *
 * The real budget is `timeoutMs` plus one probe: the pre-probe below runs before
 * `toPass` starts its own clock. Immaterial at the 5 s–30 s these sites use.
 */
export async function pollUntil<T>(
  name: string,
  probe: () => T | undefined | Promise<T | undefined>,
  opts: PollOptions,
): Promise<T> {
  return withTimedAction(null, name, async () => {
    // The FIRST probe runs OUTSIDE toPass, deliberately.
    //
    // `toPass` retries on *any* throw, not just assertion failures. Without this
    // pre-probe, a misconfigured run — `capturePathViaTool` exits non-zero and
    // throws when JAUNDER_CAPTURE_DIR is unset (capture.ts:7-10) — would turn an
    // instant, legible stack trace into a full-timeout "timed out waiting for…",
    // which points at the wrong thing entirely. Probing once up front keeps that
    // failure fast and loud; only a corruption that begins mid-run degrades to a
    // timeout, which is the rarer and less misleading case.
    const first = await probe();
    if (first !== undefined) return first;

    // `toPass` callbacks return void, so the value is captured out of band.
    let found: T | undefined;
    await expect(async () => {
      found = await probe();
      expect(found, opts.describe).not.toBeUndefined();
    }).toPass({ timeout: opts.timeoutMs, intervals: [opts.intervalMs] });
    return found as T;
  });
}

/**
 * Non-throwing sibling of {@link pollUntil}: resolves `undefined` on timeout
 * instead of throwing.
 *
 * For call sites that own their own assertion and want its diff on failure.
 * `visibility.spec.ts` is the case this exists for — it polls a feed, then
 * asserts both that the public post is present *and* that the subscribers-only
 * post is absent. A throwing wait would skip both assertions and replace a
 * content diff with a bare timeout, losing the more informative failure.
 */
export async function pollUntilOrUndefined<T>(
  name: string,
  probe: () => T | undefined | Promise<T | undefined>,
  opts: Omit<PollOptions, "describe">,
): Promise<T | undefined> {
  try {
    return await pollUntil(name, probe, { ...opts, describe: name });
  } catch {
    return undefined;
  }
}

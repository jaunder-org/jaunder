/**
 * In-app navigation for the e2e suite (#867).
 *
 * The app is a pure-CSR `leptos_router` SPA, so moving between routes in a live
 * session is a router push, not a document load. `goto` is for entering the app;
 * this is for moving once you are inside it.
 *
 * ## The barrier is the point
 *
 * `goto` hands every document load a synchronisation barrier for free — it waits
 * for `body[data-mounted]`. A router push has no equivalent: the URL changes
 * immediately and the destination renders whenever its resources resolve. Left
 * to ad-hoc waits, that difference is where flake comes from, so `expected.ready`
 * is **required** and is asserted to be meaningful: a selector that already
 * matches before the move would be a barrier that waits for nothing, and is
 * rejected rather than silently accepted.
 *
 * Do not call `waitForMount` here — the app is already mounted, so
 * `body[data-mounted]` is already present and would pass vacuously.
 */

import { expect, type Page } from "@playwright/test";
import { withTimedAction } from "./actions";
import { BASE_URL } from "./helpers";

export type InAppDestination = {
  /** The destination path, e.g. `"/app"`. */
  url: string;
  /**
   * A selector that proves the destination rendered. Must NOT already match
   * before the move — see the module doc.
   */
  ready: string;
  /** Barrier timeout. Defaults to Playwright's configured expect timeout. */
  timeoutMs?: number;
};

/**
 * Perform an in-app move and wait for the destination to settle.
 *
 * `action` is the move itself — normally a click on the control a real user
 * would use. Recorded as a `ui.navigate` action so in-app moves are as visible
 * in traces as document loads are.
 */
export async function navigateInApp(
  page: Page,
  action: () => Promise<void>,
  expected: InAppDestination,
): Promise<void> {
  const alreadyThere = await page.locator(expected.ready).count();
  if (alreadyThere > 0) {
    throw new Error(
      `navigateInApp: the ready selector ${expected.ready} already matches ` +
        `before the move, so it is a barrier that waits for nothing. Pick a ` +
        `selector unique to ${expected.url} (#867).`,
    );
  }

  await withTimedAction(page, "ui.navigate", async () => {
    await action();
    await page.waitForURL(`${BASE_URL}${expected.url}`, {
      timeout: expected.timeoutMs,
    });
    await expect(page.locator(expected.ready).first()).toBeVisible({
      timeout: expected.timeoutMs,
    });
  });
}

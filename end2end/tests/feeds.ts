/**
 * Shared feed-polling helper for the e2e suite (#794).
 *
 * "Poll a feed until it contains a marker" was written twice — a local
 * `fetchFeedContaining` in `feeds.spec.ts` and an inline loop in
 * `visibility.spec.ts` — neither instrumented. Promoted here so the wait is
 * timed once, in one place.
 *
 * The feed cache is eventually consistent: a published post is visible
 * immediately on a cache miss, but the background worker can cache an earlier
 * snapshot (e.g. between two publishes), so reads may lag until the worker
 * regenerates.
 */

import type { Page } from "@playwright/test";
import { pollUntil, pollUntilOrUndefined } from "./polling";

export type FeedResponse = { body: string; contentType: string };

/** A feed read that may not have matched. `body` is the last 200-response body
 *  seen — real content even when `matched` is false, so a caller's own
 *  assertions still produce a content diff rather than diffing against "". */
export type FeedSnapshot = FeedResponse & { matched: boolean };

/** A browser-resolved Syndication Feed discovery link from the current document. */
export type AlternateLink = { href: string; type: string };

/** Read the current document's alternate links without asserting or waiting. */
export async function readAlternateLinks(page: Page): Promise<AlternateLink[]> {
  return page.$$eval('head link[rel="alternate"]', (elements) =>
    elements.map((element) => {
      const link = element as HTMLLinkElement;
      return { href: link.href, type: link.type };
    }),
  );
}

const FEED_POLL_INTERVAL_MS = 500;

/** Per-fetch poll deadline for the eventually-consistent feed cache.
 *
 *  Exported because `feeds.spec.ts` derives its whole-test budget from this and
 *  `FORMATS.length` (#270): adding a format or changing this value must carry
 *  the budget with it — a restated number at the call site silently drifts.
 *  One definition, imported. */
export const FEED_POLL_TIMEOUT_MS = 25_000;

/**
 * Poll `url` until its body contains `marker`, returning what was last seen
 * either way.
 *
 * The last body is retained deliberately:
 * both kinds of call site need it on failure — the throwing one puts it in the
 * timeout message, and the asserting one diffs against it. Returning only
 * `undefined` on timeout would leave a caller asserting against an empty string,
 * where a `not.toContain(...)` check passes *vacuously* — a green assertion that
 * proves nothing.
 */
async function pollFeed(
  page: Page,
  url: string,
  marker: string,
  timeoutMs: number,
): Promise<FeedSnapshot> {
  let last: FeedResponse = { body: "", contentType: "" };
  const found = await pollUntilOrUndefined(
    "wait.feed",
    async () => {
      const res = await page.request.get(url);
      if (res.status() !== 200) return undefined;
      const seen = {
        body: await res.text(),
        contentType: res.headers()["content-type"] ?? "",
      };
      last = seen;
      return seen.body.includes(marker) ? seen : undefined;
    },
    { intervalMs: FEED_POLL_INTERVAL_MS, timeoutMs },
  );
  return found !== undefined
    ? { ...found, matched: true }
    : { ...last, matched: false };
}

/**
 * Poll `url` until its body contains `marker`; throw if it never does.
 *
 * For call sites whose assertion is *about* the marker being there — the
 * timeout message, which carries the last body seen, is the failure they want.
 */
export async function fetchFeedContaining(
  page: Page,
  url: string,
  marker: string,
  timeoutMs = FEED_POLL_TIMEOUT_MS,
): Promise<FeedResponse> {
  const snapshot = await pollFeed(page, url, marker, timeoutMs);
  if (!snapshot.matched) {
    throw new Error(
      `feed ${url} never contained "${marker}" within ${timeoutMs}ms; ` +
        `last body: ${snapshot.body.slice(0, 300)}`,
    );
  }
  return { body: snapshot.body, contentType: snapshot.contentType };
}

/**
 * As {@link fetchFeedContaining}, but never throws — it returns the last body
 * seen with `matched: false`.
 *
 * For call sites that make their own assertions about the body and want those
 * diffs on failure rather than a bare timeout: notably a caller asserting both
 * what the feed *does* and *does not* contain, where throwing would skip the
 * second assertion and an empty body would make it pass vacuously.
 */
export async function fetchFeedSnapshot(
  page: Page,
  url: string,
  marker: string,
  timeoutMs = FEED_POLL_TIMEOUT_MS,
): Promise<FeedSnapshot> {
  return pollFeed(page, url, marker, timeoutMs);
}

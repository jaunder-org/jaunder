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

const FEED_POLL_INTERVAL_MS = 500;

/** Per-fetch poll deadline for the eventually-consistent feed cache.
 *
 *  Exported because `feeds.spec.ts` derives its whole-test budget from this and
 *  `FORMATS.length` (#270): adding a format or changing this value must carry
 *  the budget with it, which is the coupling that had silently drifted before.
 *  One definition, imported — not restated at the call site. */
export const FEED_POLL_TIMEOUT_MS = 25_000;

async function probeFeed(
  page: Page,
  url: string,
  marker: string,
): Promise<FeedResponse | undefined> {
  const res = await page.request.get(url);
  if (res.status() !== 200) return undefined;
  const body = await res.text();
  if (!body.includes(marker)) return undefined;
  return { body, contentType: res.headers()["content-type"] ?? "" };
}

/**
 * Poll `url` until its body contains `marker`; throw if it never does.
 *
 * For call sites whose assertion is *about* the marker being there — the
 * timeout message is the failure they want.
 */
export async function fetchFeedContaining(
  page: Page,
  url: string,
  marker: string,
  timeoutMs = FEED_POLL_TIMEOUT_MS,
): Promise<FeedResponse> {
  return pollUntil("wait.feed", () => probeFeed(page, url, marker), {
    intervalMs: FEED_POLL_INTERVAL_MS,
    timeoutMs,
    describe: `feed ${url} to contain "${marker}"`,
  });
}

/**
 * As {@link fetchFeedContaining}, but resolves `undefined` on timeout.
 *
 * For call sites that make their own assertions about the body and want those
 * diffs on failure rather than a bare timeout — notably a caller that asserts
 * both what the feed *does* and *does not* contain, where throwing would skip
 * the second assertion entirely.
 */
export async function fetchFeedContainingOrUndefined(
  page: Page,
  url: string,
  marker: string,
  timeoutMs = FEED_POLL_TIMEOUT_MS,
): Promise<FeedResponse | undefined> {
  return pollUntilOrUndefined("wait.feed", () => probeFeed(page, url, marker), {
    intervalMs: FEED_POLL_INTERVAL_MS,
    timeoutMs,
  });
}

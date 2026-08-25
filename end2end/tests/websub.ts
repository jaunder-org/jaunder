/**
 * Shared WebSub-capture utilities for Jaunder e2e tests.
 *
 * When capture is on, the server uses a file-capturing WebSub client that
 * appends every hub ping as a JSON line to the websub capture file (instead of
 * contacting a real hub). These helpers read that file and wait for new pings to
 * appear — mirroring the mail-capture helpers in `mail.ts`.
 *
 * ## Usage
 *
 * Snapshot the ping count with `readPingLines().length` *before* triggering the
 * action that should produce pings. Use `waitForPingMatching` when one
 * predicate match establishes the boundary, or `waitForPingWave` when every
 * exact requested Syndication Feed URL must appear after the cursor.
 *
 * ```ts
 * const pingsBefore = readPingLines().length;
 * await publishPost(page); // triggers feed regen + hub pings
 * const ping = await waitForPingMatching(pingsBefore, isUserFeed);
 * const wave = await waitForPingWave(pingsBefore, userFeedUrls);
 * ```
 *
 * There is deliberately no count-only waiter: one publish enqueues events for
 * several Syndication Feeds, so callers must identify the relevant record or
 * complete wave (#794).
 */

import * as fs from "fs";

import { capturePathViaTool } from "./capture";
import { pollUntil } from "./polling";

// Resolved lazily and memoized via `test-support capture-path` so the filename
// convention lives only in the Rust `host` crate — never restated here.
let cachedWebsubFile: string | undefined;
function websubCaptureFile(): string {
  return (cachedWebsubFile ??= capturePathViaTool("websub"));
}

export interface CapturedPing {
  hub_url: string;
  feed_url: string;
  sent_at: string;
}

/** Return every non-empty line currently in the WebSub capture file. */
export function readPingLines(): string[] {
  if (!fs.existsSync(websubCaptureFile())) return [];
  return fs
    .readFileSync(websubCaptureFile(), "utf-8")
    .trim()
    .split("\n")
    .filter((line) => line.trim());
}

/**
 * Wait for a ping (written after `previousCount` lines) whose `feed_url`
 * matches `predicate`, then return it.
 *
 * A single publish enqueues events for several feeds (site + per-user × 3
 * formats), so the capture file gains multiple lines per mutation. Use this to
 * assert that a *specific* feed was pinged rather than just "some ping arrived".
 */
export async function waitForPingMatching(
  previousCount: number,
  predicate: (feedUrl: string) => boolean,
  timeoutMs = 30_000,
): Promise<CapturedPing> {
  // Resolve the capture path BEFORE polling. `capturePathViaTool` throws when
  // JAUNDER_CAPTURE_DIR is unset, and a throw inside the probe would be retried
  // to the full timeout instead of failing loudly (#794).
  const file = websubCaptureFile();
  return pollUntil(
    "wait.websub_ping",
    () => {
      const lines = readPingLines();
      for (let i = previousCount; i < lines.length; i++) {
        const ping = JSON.parse(lines[i]) as CapturedPing;
        if (predicate(ping.feed_url)) return ping;
      }
      return undefined;
    },
    {
      intervalMs: 250,
      timeoutMs,
      describe: `a matching WebSub ping at ${file}`,
    },
  );
}

/**
 * Find the first captured ping for every exact requested Syndication Feed URL
 * after `previousCount`, returning the records in deduplicated request order.
 */
export function findPingWave(
  lines: readonly string[],
  previousCount: number,
  feedUrls: readonly string[],
): CapturedPing[] | undefined {
  const expectedUrls = [...new Set(feedUrls)];
  const expected = new Set(expectedUrls);
  const firstByUrl = new Map<string, CapturedPing>();

  for (let i = previousCount; i < lines.length; i++) {
    const ping = JSON.parse(lines[i]) as CapturedPing;
    if (expected.has(ping.feed_url) && !firstByUrl.has(ping.feed_url)) {
      firstByUrl.set(ping.feed_url, ping);
    }
  }

  if (firstByUrl.size !== expectedUrls.length) return undefined;
  return expectedUrls.map((feedUrl) => firstByUrl.get(feedUrl)!);
}

/**
 * Wait for the first ping for every exact requested Syndication Feed URL after
 * `previousCount`, returning the captured records in request order.
 *
 * Unrelated pings are ignored. Duplicate requested URLs count once, and only
 * the first captured ping for each URL belongs to the returned wave.
 */
export async function waitForPingWave(
  previousCount: number,
  feedUrls: readonly string[],
  timeoutMs = 30_000,
): Promise<CapturedPing[]> {
  // Fail before polling when capture is unavailable rather than retrying a
  // configuration error until the timeout.
  const file = websubCaptureFile();
  return pollUntil(
    "wait.websub_ping",
    () => findPingWave(readPingLines(), previousCount, feedUrls),
    {
      intervalMs: 250,
      timeoutMs,
      describe: `a complete WebSub Publish Ping wave at ${file}`,
    },
  );
}

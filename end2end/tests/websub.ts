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
 * action that should produce a ping, then pass that count to `waitForNewPing`.
 * This prevents returning a stale ping written by a previous test.
 *
 * ```ts
 * const pingsBefore = readPingLines().length;
 * await publishPost(page); // triggers feed regen + hub ping
 * const ping = await waitForNewPing(pingsBefore);
 * ```
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
 * Wait until the capture file has more lines than `previousCount`, then return
 * the newest ping.
 *
 * The feed worker runs on a ~10s tick, so allow a generous default timeout.
 * Always pass the line count snapshotted *before* triggering the action so
 * pings written by earlier tests do not satisfy the wait.
 */
export async function waitForNewPing(
  previousCount: number,
  timeoutMs = 30_000,
): Promise<CapturedPing> {
  // Resolved before polling — see the note in `mail.ts`'s `waitForNewEmail`.
  const file = websubCaptureFile();
  return pollUntil(
    "wait.websub_ping",
    () => {
      const lines = readPingLines();
      return lines.length > previousCount
        ? (JSON.parse(lines[lines.length - 1]) as CapturedPing)
        : undefined;
    },
    {
      intervalMs: 250,
      timeoutMs,
      describe: `a new captured WebSub ping at ${file}`,
    },
  );
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
  // Resolved before polling — see the note in `mail.ts`'s `waitForNewEmail`.
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

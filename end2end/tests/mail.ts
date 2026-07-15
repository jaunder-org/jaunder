/**
 * Shared mail-capture utilities for Jaunder e2e tests.
 *
 * The server writes every outbound email as a JSON line to the mail capture file
 * when running in test mode.  These helpers read that file and wait for new
 * messages to appear.
 *
 * ## Usage
 *
 * Always prefer `waitForNewEmail(previousCount)` over any "wait for latest"
 * approach.  Snapshot the line count with `readEmailLines().length` *before*
 * triggering the action that sends the email, then pass that count to
 * `waitForNewEmail`.  This prevents returning a stale email written by a
 * previous test.
 *
 * Example:
 * ```ts
 * const emailsBefore = readEmailLines().length;
 * await page.click('button[type="submit"]'); // triggers the email
 * const email = await waitForNewEmail(emailsBefore);
 * ```
 */

import * as fs from "fs";

import { capturePathViaTool } from "./capture";

// Resolved lazily and memoized via `test-support capture-path` so the filename
// convention lives only in the Rust `host` crate — never restated here.
let cachedMailFile: string | undefined;
function mailCaptureFile(): string {
  return (cachedMailFile ??= capturePathViaTool("mail"));
}

export interface CapturedEmail {
  to: string[];
  from: string | null;
  subject: string;
  body_text: string;
}

/** Return every non-empty line currently in the mail capture file. */
export function readEmailLines(): string[] {
  if (!fs.existsSync(mailCaptureFile())) return [];
  return fs
    .readFileSync(mailCaptureFile(), "utf-8")
    .trim()
    .split("\n")
    .filter((line) => line.trim());
}

/**
 * Wait until the mail file has more lines than `previousCount`, then return
 * the newest email.
 *
 * Always pass the line count snapshotted *before* submitting the form so that
 * emails written by earlier tests in the same file do not satisfy the wait.
 */
export async function waitForNewEmail(
  previousCount: number,
  timeoutMs = 5_000,
): Promise<CapturedEmail> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const lines = readEmailLines();
    if (lines.length > previousCount) {
      return JSON.parse(lines[lines.length - 1]) as CapturedEmail;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(
    `timed out waiting for new captured email at ${mailCaptureFile()}`,
  );
}

/**
 * Extract the `token=...` value from a captured email body (verification and
 * password-reset links always carry one).  Throws if absent, so callers get a
 * clear failure instead of an opaque `undefined` downstream.
 */
export function extractToken(email: CapturedEmail): string {
  const match = email.body_text.match(/token=([^\s]+)/);
  if (!match) throw new Error("no token in captured email");
  return match[1];
}

/**
 * Extract the `invite_code=...` value from a captured invitation email body
 * (#433: the invite link is `{base_url}/register?invite_code=<code>`).  Throws
 * if absent, mirroring `extractToken`, so callers fail clearly rather than
 * navigating with an `undefined` code.
 */
export function extractInviteCode(email: CapturedEmail): string {
  const match = email.body_text.match(/invite_code=([^\s]+)/);
  if (!match) throw new Error("no invite_code in captured email");
  return match[1];
}

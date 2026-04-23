/**
 * Shared mail-capture utilities for Jaunder e2e tests.
 *
 * The server writes every outbound email as a JSON line to `MAIL_CAPTURE_FILE`
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

export const MAIL_CAPTURE_FILE =
  process.env.JAUNDER_MAIL_CAPTURE_FILE ?? "/tmp/jaunder-mail.jsonl";

export interface CapturedEmail {
  to: string[];
  from: string | null;
  subject: string;
  body_text: string;
}

/** Return every non-empty line currently in the mail capture file. */
export function readEmailLines(): string[] {
  if (!fs.existsSync(MAIL_CAPTURE_FILE)) return [];
  return fs
    .readFileSync(MAIL_CAPTURE_FILE, "utf-8")
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
    `timed out waiting for new captured email at ${MAIL_CAPTURE_FILE}`,
  );
}

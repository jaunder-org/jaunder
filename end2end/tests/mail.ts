/**
 * Shared mail-capture utilities for Jaunder e2e tests.
 *
 * The server writes every outbound email as a JSON line to the mail capture file
 * when running in test mode.  These helpers read that file and wait for new
 * messages to appear.
 *
 * ## Usage
 *
 * Wait for mail through the **`mailbox` fixture**, not by reading this file
 * directly: it is scoped to one recipient and keeps a per-test cursor, so
 * parallel tests never consume each other's mail.
 *
 * ```ts
 * test("...", async ({ mailbox }) => {
 *   await click(page, SEL.submit);          // triggers the email
 *   const email = await mailbox.waitForNewEmail();
 * });
 * ```
 *
 * A count-based `waitForNewEmail(previousCount)` used to live here. It had no
 * callers — every site uses the fixture — so it was removed rather than carried
 * forward (#794).
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

/**
 * Extract the full token-bearing link from a captured email body — whatever the
 * email actually contains: an absolute `https://…/path?token=…` (correct) or a
 * bare relative `/path?token=…` (the bug a caller can then assert against).
 * Throws if absent, so a missing link fails clearly rather than downstream.
 */
export function extractLink(email: CapturedEmail): string {
  const match = email.body_text.match(/(\S+token=\S+)/);
  if (!match) throw new Error("no token-bearing link in captured email");
  return match[1];
}

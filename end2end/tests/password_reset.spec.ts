import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import * as fs from "fs";

const MAIL_CAPTURE_FILE =
  process.env.JAUNDER_MAIL_CAPTURE_FILE ?? "/tmp/jaunder-mail.jsonl";

interface CapturedEmail {
  to: string[];
  from: string | null;
  subject: string;
  body_text: string;
}

function readEmailLines(): string[] {
  if (!fs.existsSync(MAIL_CAPTURE_FILE)) return [];
  return fs
    .readFileSync(MAIL_CAPTURE_FILE, "utf-8")
    .trim()
    .split("\n")
    .filter((l) => l.trim());
}

// Waits until the mail file has more lines than `previousCount`, then returns
// the newest email.  This avoids a race where the file already contains emails
// from a prior test (e.g. the email-verification email) and the function
// returns stale content before the expected email has been written.
async function waitForNewEmail(
  previousCount: number,
  timeoutMs = 5000,
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

async function waitForHydration(page: Page): Promise<void> {
  await page.waitForSelector("body[data-hydrated]");
}

// M3.11.13: Full password reset flow.
test("password reset flow completes successfully", async ({ page }) => {
  test.setTimeout(15_000);

  // Snapshot email count before submitting so we can detect the new email even
  // if prior tests (e.g. email verification) have already written to the file.
  const emailsBefore = readEmailLines().length;

  // Request a password reset on /forgot-password
  await page.goto("http://localhost:3000/forgot-password");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  // Page should show a neutral confirmation (not confirm whether user exists)
  await expect(page.locator("p")).toContainText(/check|sent|email/i);

  // Extract the reset token from the captured mail file, waiting for the email
  // that was sent after our form submission (not any pre-existing email).
  const email = await waitForNewEmail(emailsBefore);
  const tokenMatch = email!.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the reset link and submit a new password
  await page.goto(`http://localhost:3000/reset-password?token=${token}`);
  await waitForHydration(page);
  await page.fill('input[name="new_password"]', "resetpassword789");
  await page.click('button[type="submit"]');
  // Wait for the Leptos Router redirect to /login that fires on successful reset.
  // This ensures the server function has completed before we test the old password.
  // Using networkidle here races with Firefox: it fires before the ActionForm
  // AJAX response arrives, causing page.goto("/login") to cancel the in-flight
  // request and the password change never persists.
  await page.waitForURL("**/login");

  // Login with the old password should fail
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  // Use a generous timeout here: Firefox's networkidle may fire before the
  // ActionForm fetch response arrives under VM load, so we poll until the
  // error element appears rather than relying on networkidle as a signal.
  await expect(page.locator(".error")).toBeVisible({ timeout: 10_000 });

  // Login with new password should succeed
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "resetpassword789");
  await page.click('button[type="submit"]');
  await page.waitForSelector("a[href='/logout']", { timeout: 10_000 });
  await waitForHydration(page);
  await expect(page.locator("h1")).toHaveText("Jaunder", { timeout: 10_000 });
});

// M3.11.14: visiting /reset-password with an invalid token shows an error.
test("visiting reset-password with invalid token shows error", async ({
  page,
}) => {
  await page.goto(
    "http://localhost:3000/reset-password?token=totally_invalid_token",
  );
  await waitForHydration(page);
  await page.fill('input[name="new_password"]', "somepassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".error")).toBeVisible();
});

// M3.11.15: /forgot-password for a user with no verified email shows the "contact operator" error.
test("forgot-password for user without verified email shows contact operator error", async ({
  page,
}) => {
  await page.goto("http://localhost:3000/forgot-password");
  await waitForHydration(page);
  // "testnoemail" user should exist but have no verified email
  await page.fill('input[name="username"]', "testnoemail");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".error")).toBeVisible();
});

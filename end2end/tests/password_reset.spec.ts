import { test, expect, type Page } from "@playwright/test";
import * as fs from "fs";

const MAIL_CAPTURE_FILE =
  process.env.JAUNDER_MAIL_CAPTURE_FILE ?? "/tmp/jaunder-mail.jsonl";

interface CapturedEmail {
  to: string[];
  from: string | null;
  subject: string;
  body_text: string;
}

function readLatestEmail(): CapturedEmail | null {
  if (!fs.existsSync(MAIL_CAPTURE_FILE)) return null;
  const content = fs.readFileSync(MAIL_CAPTURE_FILE, "utf-8");
  const lines = content
    .trim()
    .split("\n")
    .filter((l) => l.trim());
  if (lines.length === 0) return null;
  return JSON.parse(lines[lines.length - 1]) as CapturedEmail;
}

async function waitForHydration(page: Page): Promise<void> {
  await page.waitForSelector("body[data-hydrated]");
}

// M3.11.13: Full password reset flow.
test("password reset flow completes successfully", async ({ page }) => {
  // Request a password reset on /forgot-password
  await page.goto("http://localhost:3000/forgot-password");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  // Page should show a neutral confirmation (not confirm whether user exists)
  await expect(page.locator("p")).toContainText(/check|sent|email/i);

  // Extract the reset token from the captured mail file
  const email = readLatestEmail();
  expect(email).not.toBeNull();
  const tokenMatch = email!.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the reset link and submit a new password
  await page.goto(`http://localhost:3000/reset-password?token=${token}`);
  await waitForHydration(page);
  await page.fill('input[name="new_password"]', "resetpassword789");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");

  // Login with the old password should fail
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");
  await expect(page.locator(".error")).toBeVisible();

  // Login with new password should succeed
  await page.goto("http://localhost:3000/login");
  await waitForHydration(page);
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "resetpassword789");
  await page.click('button[type="submit"]');
  await page.waitForLoadState("networkidle");
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator("h1")).toHaveText("Welcome to Leptos!");
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

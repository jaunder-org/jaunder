import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { goto, click, waitForSelector, waitForHydration } from "./helpers";
import { readEmailLines, waitForNewEmail } from "./mail";

// M3.11.13: Full password reset flow.
test("password reset flow completes successfully", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));

  // Snapshot email count before submitting so we can detect the new email even
  // if prior tests (e.g. email verification) have already written to the file.
  const emailsBefore = readEmailLines().length;

  // Request a password reset on /forgot-password
  await goto(page, "/forgot-password");
  await page.fill('input[name="username"]', "testlogin");
  await click(page, 'button[type="submit"]');

  // Page should show a neutral confirmation (not confirm whether user exists).
  await expect(page.locator("p")).toContainText(/check|sent|email/i, {
    timeout: 10_000,
  });

  // Extract the reset token from the captured mail file, waiting for the email
  // that was sent after our form submission (not any pre-existing email).
  const email = await waitForNewEmail(emailsBefore);
  const tokenMatch = email.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the reset link and submit a new password
  await goto(page, `/reset-password?token=${token}`);
  await page.fill('input[name="new_password"]', "resetpassword789");
  await click(page, 'button[type="submit"]');
  // Wait for the router redirect to /login — ensures the password change has
  // persisted before testing the old credential below.
  await page.waitForURL("**/login");

  // Login with the old password should fail
  await goto(page, "/login");
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "testpassword123");
  await click(page, 'button[type="submit"]');
  await expect(page.locator(".error")).toBeVisible({ timeout: 10_000 });

  // Login with new password should succeed from the same hydrated login page.
  await page.fill('input[name="username"]', "");
  await page.fill('input[name="password"]', "");
  await page.fill('input[name="username"]', "testlogin");
  await page.fill('input[name="password"]', "resetpassword789");
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, "a[href='/logout']", { timeout: 10_000 });
  await waitForHydration(page);
  await expect(page.locator(".j-topbar h1")).toHaveText("Home", {
    timeout: 10_000,
  });
});

// M3.11.14: visiting /reset-password with an invalid token shows an error.
test("visiting reset-password with invalid token shows error", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await goto(page, "/reset-password?token=totally_invalid_token");
  await page.fill('input[name="new_password"]', "somepassword123");
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, ".error");
  await expect(page.locator(".error")).toBeVisible();
});

// M3.11.15: /forgot-password for a user with no verified email shows the "contact operator" error.
test("forgot-password for user without verified email shows contact operator error", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await goto(page, "/forgot-password");
  // "testnoemail" user should exist but have no verified email
  await page.fill('input[name="username"]', "testnoemail");
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, ".error");
  await expect(page.locator(".error")).toBeVisible();
});

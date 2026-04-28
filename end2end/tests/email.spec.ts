import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { goto, login } from "./helpers";
import { readEmailLines, waitForNewEmail } from "./mail";

// M3.10.11: Full email verification flow.
test("email verification flow completes successfully", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));

  await login(page, "testlogin", "testpassword123");

  // Navigate to email settings and submit an address
  await goto(page, "/profile/email");

  // Snapshot email count before submitting so we don't consume a stale email.
  const emailsBefore = readEmailLines().length;

  await page.fill('input[name="email"]', "testlogin@example.com");
  await page.click('button[type="submit"]');

  await expect(page.locator('p:has-text("Check your email")')).toBeVisible({
    timeout: 10_000,
  });

  // Extract the verification token from the captured mail file, waiting for the
  // email that was sent after our form submission (not any pre-existing email).
  const email = await waitForNewEmail(emailsBefore);
  const tokenMatch = email.body_text.match(/token=([^\s]+)/);
  expect(tokenMatch).not.toBeNull();
  const token = tokenMatch![1];

  // Visit the verification link
  await goto(page, `/verify-email?token=${token}`);
  await expect(page.locator('p:has-text("verified")')).toBeVisible();

  // Confirm email is shown as verified on the profile page
  await goto(page, "/profile/email");
  await expect(page.locator("p")).toContainText("verified");
});

// M3.10.12: Invalid token shows an error.
test("visiting verify-email with invalid token shows error", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));
  await goto(page, "/verify-email?token=totally_invalid_token");
  await expect(page.locator(".error")).toBeVisible();
});

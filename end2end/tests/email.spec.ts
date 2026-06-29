import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { goto, login } from "./helpers";

// M3.10.11: Full email verification flow.
test("email verification flow completes successfully", async ({
  page,
  user,
  mailbox,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));

  await login(page, user.username, user.password);

  // Navigate to email settings and submit this user's unique address.
  await goto(page, "/profile/email");
  await page.fill('input[name="email"]', user.email);
  await page.click('button[type="submit"]');

  await expect(page.locator('p:has-text("Check your email")')).toBeVisible({
    timeout: 10_000,
  });

  // Read this recipient's verification mail (recipient-scoped, parallel-safe).
  const email = await mailbox.waitForNewEmail();
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

import { test, expect } from "./fixtures";
import { goto, login, click, expectFlash } from "./helpers";
import { SEL } from "./selectors";
import { extractToken } from "./mail";

// M3.10.11: Full email verification flow.
test("email verification flow completes successfully", async ({
  page,
  user,
  mailbox,
}) => {
  await login(page, user.username, user.password);

  // Navigate to email settings and submit this user's unique address.
  await goto(page, "/profile/email");
  await page.fill('input[name="email"]', user.email);
  await click(page, SEL.submit);

  await expectFlash(page, "Check your email", 10_000);

  // Read this recipient's verification mail (recipient-scoped, parallel-safe).
  const email = await mailbox.waitForNewEmail();
  const token = extractToken(email);

  // Visit the verification link
  await goto(page, `/verify-email?token=${token}`);
  await expectFlash(page, "verified");

  // Confirm email is shown as verified on the profile page
  await goto(page, "/profile/email");
  await expect(page.locator("p")).toContainText("verified");
});

// M3.10.12: Invalid token shows an error.
test("visiting verify-email with invalid token shows error", async ({
  page,
}) => {
  await goto(page, "/verify-email?token=totally_invalid_token");
  await expect(page.locator(SEL.error)).toBeVisible();
});

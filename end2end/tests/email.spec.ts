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

// #397: the email field is client-validated (ValidatedInput<Email>, ADR-0065) —
// submit is gated disable-until-valid and a malformed address shows an inline error,
// so a bad value never reaches the typed `#[server]` arg.
test("email form gates submit until a valid address is entered", async ({
  page,
  user,
}) => {
  await login(page, user.username, user.password);
  await goto(page, "/profile/email");

  const emailInput = page.locator('input[name="email"]');

  // Pristine empty field: invalid, so submit is disabled and no error is shown yet.
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await expect(page.locator(SEL.error)).not.toBeVisible();

  // A malformed address, once the field is touched (blur), shows the inline
  // client-local error and keeps submit disabled.
  await emailInput.fill("not-an-email");
  await emailInput.blur();
  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();

  // A valid address clears the error and enables submit.
  await emailInput.fill(user.email);
  await expect(page.locator(SEL.error)).not.toBeVisible();
  await expect(page.locator(SEL.submit)).toBeEnabled();
});

// M3.10.12: Invalid token shows an error.
test("visiting verify-email with invalid token shows error", async ({
  page,
}) => {
  await goto(page, "/verify-email?token=totally_invalid_token");
  await expect(page.locator(SEL.error)).toBeVisible();
});

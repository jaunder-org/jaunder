import { test, expect } from "./fixtures";
import {
  goto,
  click,
  waitForSelector,
  waitForMount,
  fillLoginForm,
  followEmailLink,
  requestPasswordReset,
  stallServerFn,
} from "./helpers";
import { SEL } from "./selectors";

// M3.11.13: Full password reset flow.
test("password reset flow completes successfully", async ({
  page,
  verifiedUser,
  mailbox,
}) => {
  // Request a password reset for this test's own verified user.
  await requestPasswordReset(page, verifiedUser.username);

  // Page should show a neutral confirmation (not confirm whether user exists).
  await expect(page.locator("p")).toContainText(/check|sent|email/i);

  // Read this recipient's reset mail (recipient-scoped, parallel-safe) and follow
  // the emitted link — asserting it is absolute, so a relative-link regression fails.
  const email = await mailbox.waitForNewEmail();
  await followEmailLink(page, email, "/reset-password");
  await page.fill('input[name="new_password"]', "resetpassword789");
  await click(page, SEL.submit);
  // Wait for the router redirect to /login — ensures the password change has
  // persisted before testing the old credential below.
  await page.waitForURL("**/login");

  // Login with the OLD password should fail. The router has already landed on
  // /login above, so the assertions run where it landed — a full `goto` here
  // would reload the page the app just navigated to (#867).
  await waitForSelector(page, SEL.username);
  // Holdout (spec D6): a reset password logs in through the real form (and the
  // old one fails) — the form IS the subject here.
  await fillLoginForm(page, verifiedUser.username, verifiedUser.password);
  await expect(page.locator(SEL.error)).toBeVisible();

  // Login with new password should succeed from the same mounted login page.
  await page.fill(SEL.username, "");
  await page.fill(SEL.password, "");
  await fillLoginForm(page, verifiedUser.username, "resetpassword789");
  await waitForSelector(page, SEL.logoutLink, { timeout: 10_000 });
  await waitForMount(page);
  // Login redirects to `/`, now the enhanced public Local timeline (#181, D10).
  await expect(page.locator(SEL.topbarHeading)).toHaveText("jaunder.local");
});

// M3.11.14: visiting /reset-password with an invalid token shows an error.
test("visiting reset-password with invalid token shows error", async ({
  page,
}) => {
  await goto(page, "/reset-password?token=totally_invalid_token");
  await page.fill(SEL.newPassword, "somepassword123");
  await click(page, SEL.submit);
  await waitForSelector(page, SEL.error);
  await expect(page.locator(SEL.error)).toBeVisible();
});

// #410: the new-password field validates client-side via ValidatedInput<Password>.
test("reset-password rejects a too-short password client-side", async ({
  page,
}) => {
  await goto(page, "/reset-password?token=any_token"); // token irrelevant; never submitted
  const pw = page.locator(SEL.newPassword);
  await pw.fill("short"); // < 8 chars
  await pw.blur(); // touched → message shows

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();

  await pw.fill("longenough123");
  await expect(page.locator(SEL.submit)).toBeEnabled();
});

test("reset confirmation invalid input does not dispatch", async ({ page }) => {
  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/password_reset/confirm")) requests += 1;
  });
  await goto(page, "/reset-password?token=bad!token");
  const password = page.locator(SEL.newPassword);
  await password.fill("short");
  await password.blur();

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await password.press("Enter");
  expect(requests).toBe(0);

  await password.fill("longenough123");
  await expect(page.locator(SEL.error)).toHaveCount(0);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await password.press("Enter");
  expect(requests).toBe(0);
});

test("reset confirmation pending prevents duplicate dispatch", async ({
  page,
  verifiedUser,
  mailbox,
}) => {
  await requestPasswordReset(page, verifiedUser.username);
  const email = await mailbox.waitForNewEmail();
  await followEmailLink(page, email, "/reset-password");

  let requests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/password_reset/confirm")) requests += 1;
  });
  const release = await stallServerFn(page, "password_reset/confirm");
  const password = page.locator(SEL.newPassword);
  await password.fill("pendingpassword123");
  await click(page, SEL.submit);
  await expect.poll(() => requests).toBe(1);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await password.press("Enter");
  expect(requests).toBe(1);
  release();

  await page.waitForURL("**/login");
});

// M3.11.15: /forgot-password for a user with no verified email shows the
// "contact operator" error.
test("forgot-password for user without verified email shows contact operator error", async ({
  page,
  user,
}) => {
  // A freshly-registered user exists but has no verified email.
  await requestPasswordReset(page, user.username);
  await waitForSelector(page, SEL.error);
  await expect(page.locator(SEL.error)).toBeVisible();
});

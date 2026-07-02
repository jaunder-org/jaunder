import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { goto, click, waitForSelector, waitForHydration } from "./helpers";

// M3.11.13: Full password reset flow.
test("password reset flow completes successfully", async ({
  page,
  verifiedUser,
  mailbox,
}, testInfo) => {
  // Larger budget than the old seeded-account flow: `verifiedUser` self-
  // provisions a verified account out-of-band (register → login → set-email →
  // verify) before this body's forgot → reset → login-twice flow — roughly
  // double the page ops of the original pre-seeded test.
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));

  // Request a password reset for this test's own verified user.
  await goto(page, "/forgot-password");
  await page.fill('input[name="username"]', verifiedUser.username);
  await click(page, 'button[type="submit"]');

  // Page should show a neutral confirmation (not confirm whether user exists).
  await expect(page.locator("p")).toContainText(/check|sent|email/i, {
    timeout: 10_000,
  });

  // Read this recipient's reset mail (recipient-scoped, parallel-safe).
  const email = await mailbox.waitForNewEmail();
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

  // Login with the OLD password should fail
  await goto(page, "/login");
  await page.fill('input[name="username"]', verifiedUser.username);
  await page.fill('input[name="password"]', verifiedUser.password);
  await click(page, 'button[type="submit"]');
  await expect(page.locator(".error")).toBeVisible({ timeout: 10_000 });

  // Login with new password should succeed from the same hydrated login page.
  await page.fill('input[name="username"]', "");
  await page.fill('input[name="password"]', "");
  await page.fill('input[name="username"]', verifiedUser.username);
  await page.fill('input[name="password"]', "resetpassword789");
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, "a[href='/logout']", { timeout: 10_000 });
  await waitForHydration(page);
  // Login redirects to `/`, now the enhanced public Local timeline (#181, D10).
  await expect(page.locator(".j-topbar h1")).toHaveText("jaunder.local");
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

// M3.11.15: /forgot-password for a user with no verified email shows the
// "contact operator" error.
test("forgot-password for user without verified email shows contact operator error", async ({
  page,
  user,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 12_000));
  await goto(page, "/forgot-password");
  // A freshly-registered user exists but has no verified email.
  await page.fill('input[name="username"]', user.username);
  await click(page, 'button[type="submit"]');
  await waitForSelector(page, ".error");
  await expect(page.locator(".error")).toBeVisible();
});

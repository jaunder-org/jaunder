import type { BrowserContext, Page } from "@playwright/test";

import { allowSecondBoot } from "./bootBudget";
import { reenterAdminSettings } from "./admin-settings";
import { test, expect, setTestBudget } from "./fixtures";
import {
  BASE_URL,
  click,
  fillLoginForm,
  followEmailLink,
  goto,
  requestPasswordReset,
  signInAs,
  setAndVerifyEmail,
  waitForSelector,
} from "./helpers";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";
import { applySeededSession, seedConfigViaTool } from "./seed";
import { installVirtualAuthenticator } from "./virtual-authenticator";

const PASSKEY_PAGE = '[data-test="passkeys-page"]';
const PASSKEYS_LINK = '[data-test="passkeys-nav-link"]';
const PASSKEY_LOGIN = '[data-test="passkey-login-button"]';
const PASSKEYS_LIST = '[data-test="passkey-credentials-list"]';

async function exposeNonSuccessfulWebAuthn(page: Page): Promise<void> {
  await page.addInitScript(() => {
    class MockPublicKeyCredential {
      static parseCreationOptionsFromJSON(options: unknown): unknown {
        return options;
      }

      static parseRequestOptionsFromJSON(options: unknown): unknown {
        return options;
      }

      toJSON(): object {
        return {};
      }
    }

    Object.defineProperty(window, "PublicKeyCredential", {
      configurable: true,
      value: MockPublicKeyCredential,
    });
    const credentials = navigator.credentials ?? {};
    Object.defineProperty(credentials, "create", {
      configurable: true,
      value: () =>
        Promise.reject(new DOMException("Not implemented.", "NotAllowedError")),
    });
    Object.defineProperty(credentials, "get", {
      configurable: true,
      value: () =>
        Promise.reject(
          new DOMException(
            "The authenticator request was cancelled.",
            "NotAllowedError",
          ),
        ),
    });
    if (navigator.credentials === undefined) {
      Object.defineProperty(navigator, "credentials", {
        configurable: true,
        value: credentials,
      });
    }
  });
}

// The only tests that make a passkey ceremony successful use Chrome's CDP virtual
// authenticator. Firefox and WebKit retain their real-browser UI coverage below;
// neither gets a successful-ceremony mock.
test("Chromium passkeys enroll, authenticate discoverably, refresh metadata, and revoke sessions", async ({
  page,
  tracedContext,
  user,
  mailbox,
  browserName,
}, testInfo) => {
  setTestBudget(90_000);
  test.skip(
    browserName !== "chromium" ||
      testInfo.project.name !== "chromium-admin-site",
    "real resident-key ceremonies run in the serial Chromium site-settings project",
  );

  let authenticator = await installVirtualAuthenticator(page.context(), page);
  let siblingContext: BrowserContext | undefined;
  let resetContext: BrowserContext | undefined;
  let settingsContext: BrowserContext | undefined;
  let emailContext: BrowserContext | undefined;

  try {
    siblingContext = await tracedContext();
    resetContext = await tracedContext();
    settingsContext = await tracedContext();
    const settingsPage = await settingsContext.newPage();
    await signInAs(settingsPage, "testoperator");
    await goto(settingsPage, "/admin/site");
    await waitForSelector(settingsPage, 'input[name="base_url"]');
    await settingsPage.fill('input[name="base_url"]', BASE_URL);
    await click(settingsPage, 'button:has-text("Save Site Settings")');
    await waitForSelector(settingsPage, "[data-settings-saved]");
    await reenterAdminSettings(settingsPage, "site");
    await expect(settingsPage.locator('input[name="base_url"]')).toHaveValue(
      new URL(BASE_URL).toString(),
    );
    emailContext = await tracedContext();
    const emailPage = await emailContext.newPage();
    await applySeededSession(emailContext, user);
    await setAndVerifyEmail(emailPage, user.email, mailbox);
    await emailContext.close();
    emailContext = undefined;
    await signInAs(page, user.username, "Passkey current session");
    await goto(page, "/passkeys");
    await waitForSelector(page, '[data-test="passkey-registration"]');

    await page.fill('input[name="passkey-label"]', "Primary laptop");
    await page.fill('input[name="passkey-password"]', user.password);
    await click(page, '[data-test="passkey-register"]');
    await waitForSelector(page, '[data-test="passkey-registration-status"]');
    await expect(
      page.locator('[data-test="passkey-registration-status"]'),
    ).toHaveText("Passkey added.");
    await expect(page.locator(PASSKEYS_LIST)).toContainText("Primary laptop");

    const primaryCredentials = await authenticator.credentials();
    expect(primaryCredentials).toHaveLength(1);
    expect(primaryCredentials[0]?.isResidentCredential).toBe(true);

    // The primary authenticator must establish a real cookie-backed session
    // through discoverable authentication before it is replaced by the backup.
    await click(page, SEL.logoutLink);
    await page.waitForURL(`${BASE_URL}/`);
    allowSecondBoot(
      page,
      "the cold login page is the entry point for discoverable passkey authentication",
    );
    await goto(page, "/login");
    await waitForSelector(page, PASSKEY_LOGIN);
    await click(page, PASSKEY_LOGIN);
    await waitForSelector(page, SEL.logoutLink);
    await navigateInApp(page, () => click(page, PASSKEYS_LINK), {
      url: "/passkeys",
      ready: PASSKEYS_LIST,
    });
    await expect(
      page.locator('[data-test="passkey-credential"]', {
        hasText: "Primary laptop",
      }),
    ).toContainText(/Last used: (?!Never)/);

    // CDP's -1 sentinel makes every following assertion report a signed zero
    // counter. The prior assertion advanced the durable high-water mark, so this
    // proves the server accepts a cryptographically valid counter anomaly.
    await authenticator.setCredentialSignCount(primaryCredentials[0]!, -1);
    await click(page, SEL.logoutLink);
    await page.waitForURL(`${BASE_URL}/`);
    allowSecondBoot(
      page,
      "a fresh login document drives the signed counter-anomaly assertion",
    );
    await goto(page, "/login");
    await waitForSelector(page, PASSKEY_LOGIN);
    await click(page, PASSKEY_LOGIN);
    await waitForSelector(page, SEL.logoutLink);
    await navigateInApp(page, () => click(page, PASSKEYS_LINK), {
      url: "/passkeys",
      ready: PASSKEYS_LIST,
    });
    await expect(
      page.locator('[data-test="passkey-credential"]', {
        hasText: "Primary laptop",
      }),
    ).toContainText(/Last used: (?!Never)/);
    // CTAP2 excludes a second credential for the same user/RP on one
    // authenticator. A different resident authenticator models the user's
    // backup device and remains attached for the remaining assertions.
    await authenticator.dispose();
    authenticator = await installVirtualAuthenticator(page.context(), page);

    await page.fill('input[name="passkey-label"]', "Backup security key");
    await page.fill('input[name="passkey-password"]', user.password);
    await click(page, '[data-test="passkey-register"]');
    await expect(page.locator(PASSKEYS_LIST)).toContainText(
      "Backup security key",
    );
    await expect(page.locator('[data-test="passkey-credential"]')).toHaveCount(
      2,
    );

    const siblingPage = await siblingContext.newPage();
    await signInAs(siblingPage, user.username, "Passkey sibling session");
    await goto(siblingPage, "/");
    await waitForSelector(siblingPage, SEL.logoutLink);

    const primary = page.locator('[data-test="passkey-credential"]', {
      hasText: "Primary laptop",
    });
    await primary
      .locator('input[name="passkey-delete-password"]')
      .fill(user.password);
    await click(
      page,
      '[data-test="passkey-credential"]:has-text("Primary laptop") [data-test="passkey-delete"]',
    );
    await expect(page.locator(PASSKEYS_LIST)).not.toContainText(
      "Primary laptop",
    );
    await expect(page.locator(SEL.logoutLink)).toBeVisible();

    // A sibling tab retains its client-side SessionContext until a server
    // boundary. A document navigation proves the durable Session was revoked.
    allowSecondBoot(
      siblingPage,
      "a fresh protected-route load reconciles the remotely revoked sibling session",
    );
    await goto(siblingPage, "/app");
    await siblingPage.waitForURL(`${BASE_URL}/login`);
    await waitForSelector(siblingPage, SEL.username);
    await expect(siblingPage.locator(SEL.logoutLink)).toHaveCount(0);

    const backupCredentials = await authenticator.credentials();
    expect(backupCredentials).toHaveLength(1);
    expect(backupCredentials[0]?.isResidentCredential).toBe(true);

    // A document navigation keeps the target-scoped CDP authenticator attached
    // while proving its resident credential identifies the account without an
    // allow-list.
    await click(page, SEL.logoutLink);
    await page.waitForURL(`${BASE_URL}/`);
    allowSecondBoot(
      page,
      "a fresh login document proves the backup authenticator identifies the account",
    );
    await goto(page, "/login");
    await waitForSelector(page, PASSKEY_LOGIN);
    await click(page, PASSKEY_LOGIN);
    await waitForSelector(page, SEL.logoutLink);

    await navigateInApp(page, () => click(page, PASSKEYS_LINK), {
      url: "/passkeys",
      ready: PASSKEYS_LIST,
    });
    await expect(
      page.locator('[data-test="passkey-credential"]', {
        hasText: "Backup security key",
      }),
    ).toContainText(/Last used: (?!Never)/);

    const resetPage = await resetContext.newPage();
    await requestPasswordReset(resetPage, user.email);
    const email = await mailbox.waitForNewEmail();
    await followEmailLink(resetPage, email, "/reset-password", BASE_URL);
    await resetPage.fill(SEL.newPassword, "passkeyresetpassword789");
    await click(resetPage, SEL.submit);
    await resetPage.waitForURL("**/login");
    await waitForSelector(resetPage, SEL.username);

    // Reset revokes this passkey session. The same page can then establish a
    // new session through the remaining resident credential.
    await navigateInApp(page, () => click(page, '.j-nav a[href="/app"]'), {
      url: "/login",
      ready: SEL.username,
    });
    await click(page, PASSKEY_LOGIN);
    await waitForSelector(page, SEL.logoutLink);
    await navigateInApp(page, () => click(page, PASSKEYS_LINK), {
      url: "/passkeys",
      ready: PASSKEYS_LIST,
    });
    const backup = page.locator('[data-test="passkey-credential"]', {
      hasText: "Backup security key",
    });
    await backup
      .locator('input[name="passkey-delete-password"]')
      .fill("passkeyresetpassword789");
    await click(
      page,
      '[data-test="passkey-credential"]:has-text("Backup security key") [data-test="passkey-delete"]',
    );
    await waitForSelector(page, '[data-test="passkey-credentials-empty"]');

    await settingsPage.fill('input[name="base_url"]', "https://example.com");
    await click(settingsPage, 'button:has-text("Save Site Settings")');
    await reenterAdminSettings(settingsPage, "site");
    await expect(settingsPage.locator('input[name="base_url"]')).toHaveValue(
      "https://example.com/",
    );
  } finally {
    await seedConfigViaTool("site.base_url", "https://example.com").catch(
      () => undefined,
    );
    await settingsContext?.close();
    await resetContext?.close();
    await siblingContext?.close();
    await authenticator.dispose();
    await emailContext?.close();
  }
});

test("passkeys require authentication", async ({ page }) => {
  await goto(page, "/passkeys");
  await waitForSelector(page, '[data-test="passkeys-auth-required"]');
  await expect(page.locator('[data-test="passkeys-auth-required"]')).toHaveText(
    "Sign in to manage passkeys.",
  );
});

test("authenticated sidebar reaches passkeys", async ({ page, user }) => {
  await signInAs(page, user.username);
  await goto(page, "/app");
  await navigateInApp(page, () => click(page, PASSKEYS_LINK), {
    url: "/passkeys",
    ready: PASSKEY_PAGE,
  });
  await expect(page.locator(PASSKEY_PAGE)).toBeVisible();
});

test("passkey registration validates label and current password before dispatch", async ({
  page,
  user,
}) => {
  await signInAs(page, user.username);
  await exposeNonSuccessfulWebAuthn(page);
  await goto(page, "/passkeys");
  await waitForSelector(page, '[data-test="passkey-registration"]');

  const label = page.locator('input[name="passkey-label"]');
  const password = page.locator('input[name="passkey-password"]');
  const submit = page.locator('[data-test="passkey-register"]');
  await expect(submit).toBeDisabled();
  await label.focus();
  await label.fill("");
  await label.blur();
  await expect(page.locator(SEL.error)).toBeVisible();
  await label.fill("Laptop");
  await password.fill("short");
  await password.blur();
  await expect(submit).toBeDisabled();
});

test("password login remains available when passkeys are unsupported", async ({
  page,
  user,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "PublicKeyCredential", {
      configurable: true,
      value: undefined,
    });
    Object.defineProperty(navigator, "credentials", {
      configurable: true,
      value: undefined,
    });
  });
  await goto(page, "/login");
  await waitForSelector(page, '[data-test="passkey-login-unsupported"]');
  await expect(
    page.locator('[data-test="passkey-login-unsupported"]'),
  ).toHaveText("Passkeys are not supported by this browser.");

  await fillLoginForm(page, user.username, user.password);
  await waitForSelector(page, SEL.logoutLink);
});

test("passkey login reports a browser-authenticator cancellation", async ({
  page,
}) => {
  await exposeNonSuccessfulWebAuthn(page);
  await goto(page, "/login");
  await waitForSelector(page, PASSKEY_LOGIN);
  await expect(page.locator(PASSKEY_LOGIN)).toBeEnabled();
  await click(page, PASSKEY_LOGIN);
  await waitForSelector(page, '[data-test="passkey-login-status"]');
  await expect(page.locator('[data-test="passkey-login-status"]')).toHaveText(
    "Passkey sign-in cancelled.",
  );
});

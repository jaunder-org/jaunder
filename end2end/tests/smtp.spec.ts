import type { Page } from "@playwright/test";
import { expectAccessible } from "./accessibility";
import { reenterAdminSettings } from "./admin-settings";
import { bootCount, trackBoots } from "./bootBudget";
import { test, expect } from "./fixtures";
import { goto, signInAs, stallServerFn, waitForSelector } from "./helpers";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";

test.describe.configure({ mode: "serial" });

const smtpLink = 'a.j-nav-item[href="/admin/smtp"]';
const saveButton = 'button:has-text("Save SMTP Settings")';

async function expectNoStoredSecret(page: Page, secret: string) {
  const browserStorage = await page.evaluate(() => ({
    local: Object.entries(localStorage),
    session: Object.entries(sessionStorage),
  }));
  expect(JSON.stringify(browserStorage)).not.toContain(secret);
}

async function saveSmtp(page: Page) {
  const response = page.waitForResponse(
    (candidate) =>
      new URL(candidate.url()).pathname === "/api/smtp/update_settings",
  );
  await page.locator(saveButton).click();
  expect((await response).ok()).toBe(true);
  await expect(page.locator(".j-settings-saved")).toBeVisible();
}

async function disableSmtpIfEnabled(page: Page) {
  if (new URL(page.url()).pathname === "/admin/smtp") {
    await navigateInApp(
      page,
      () => page.click('a.j-nav-item[href="/admin/site"]'),
      {
        url: "/admin/site",
        ready: 'input[name="title"]',
      },
    );
  }
  await navigateInApp(page, () => page.click(smtpLink), {
    url: "/admin/smtp",
    ready: 'input[name="enabled"]',
  });
  const enabled = page.locator('input[name="enabled"]');
  await expect(enabled).toBeVisible();
  if (await enabled.isChecked()) {
    await enabled.uncheck();
    await saveSmtp(page);
  }
}

test("anonymous navigation hides SMTP and direct access is denied", async ({
  page,
}) => {
  await goto(page, "/admin/smtp");
  await expect(page.locator(smtpLink)).toHaveCount(0);
  await expect(page.locator(SEL.error)).toBeVisible();
});

test("member navigation hides SMTP and direct access is denied", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/admin/smtp");
  await expect(page.locator(smtpLink)).toHaveCount(0);
  await expect(page.locator(SEL.error)).toBeVisible();
});

test("operator manages the complete SMTP relay lifecycle in one document", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  trackBoots(page);
  await goto(page, "/admin/site");
  await waitForSelector(page, smtpLink);

  await navigateInApp(page, () => page.click(smtpLink), {
    url: "/admin/smtp",
    ready: 'input[name="enabled"]',
  });

  const enabled = page.locator('input[name="enabled"]');
  const host = page.locator('input[name="host"]');
  const port = page.locator('input[name="port"]');
  const tlsMode = page.locator('select[name="tls_mode"]');
  const sender = page.locator('input[name="sender"]');
  const authentication = page.locator('input[name="authentication_enabled"]');
  const username = page.locator('input[name="username"]');
  const password = page.locator('input[name="password"]');

  try {
    await expect(enabled).not.toBeChecked();
    await expect(host).toHaveValue("");
    await expect(port).toHaveValue("587");
    await expect(tlsMode).toHaveValue("starttls");
    await expect(sender).toHaveValue("Jaunder <noreply@localhost>");
    await expect(password).toHaveAttribute("type", "password");
    await expect(password).toHaveValue("");
    await expectAccessible(page);

    await enabled.check();
    await expect(page.locator(saveButton)).toBeDisabled();
    await host.fill("relay.example.com");
    await port.fill("0");
    await port.blur();
    await expect(page.locator(".j-site-form .error")).toContainText(
      "port must not be zero",
    );
    await expect(page.locator(saveButton)).toBeDisabled();
    await port.fill("2525");
    await tlsMode.selectOption("tls");
    await sender.fill("not-a-mailbox");
    await sender.blur();
    await expect(page.locator(".j-site-form .error")).toContainText(
      "must be an email address",
    );
    await expect(page.locator(saveButton)).toBeDisabled();
    await sender.fill("Jaunder Mail <mail@example.com>");
    await authentication.check();
    await expect(page.locator(saveButton)).toBeDisabled();
    await username.fill("relay-user");
    await expect(page.locator(saveButton)).toBeDisabled();

    const firstSecret = "first-relay-secret";
    await password.fill(firstSecret);
    await expect(page.locator(saveButton)).toBeEnabled();
    await expectNoStoredSecret(page, firstSecret);
    const release = await stallServerFn(page, "smtp/update_settings");
    await page.locator(saveButton).click();
    await expect(password).toHaveValue("");
    await expectNoStoredSecret(page, firstSecret);
    release();
    await expect(page.locator(".j-settings-saved")).toContainText(
      "Restart Jaunder through its service manager",
    );

    await reenterAdminSettings(page, "smtp");
    await expect(page.locator('input[name="enabled"]')).toBeChecked();
    await expect(page.locator('input[name="host"]')).toHaveValue(
      "relay.example.com",
    );
    await expect(
      page.locator('input[name="authentication_enabled"]'),
    ).toBeChecked();
    await expect(page.locator('input[name="username"]')).toHaveValue(
      "relay-user",
    );
    await expect(page.locator('input[name="password"]')).toHaveValue("");
    await expect(page.getByText("A password is configured")).toBeVisible();
    expect(await page.content()).not.toContain(firstSecret);

    // A blank password keeps the stored password while other settings change.
    await page.locator('input[name="host"]').fill("kept.example.com");
    await saveSmtp(page);
    await reenterAdminSettings(page, "smtp");
    await expect(page.locator('input[name="host"]')).toHaveValue(
      "kept.example.com",
    );
    await expect(page.getByText("A password is configured")).toBeVisible();

    // A non-empty password replaces the stored password and is cleared on dispatch.
    const replacement = "replacement-relay-secret";
    await page.locator('input[name="password"]').fill(replacement);
    const replacementResponse = page.waitForResponse(
      (candidate) =>
        new URL(candidate.url()).pathname === "/api/smtp/update_settings",
    );
    await page.locator(saveButton).click();
    await expect(page.locator('input[name="password"]')).toHaveValue("");
    expect((await replacementResponse).ok()).toBe(true);
    await expect(page.locator(".j-settings-saved")).toBeVisible();
    await expectNoStoredSecret(page, replacement);
    await reenterAdminSettings(page, "smtp");
    await expect(page.getByText("A password is configured")).toBeVisible();
    expect(await page.content()).not.toContain(replacement);

    // Authentication is a pair: turning it off removes username and password.
    await page.locator('input[name="authentication_enabled"]').uncheck();
    await saveSmtp(page);
    await reenterAdminSettings(page, "smtp");
    await expect(
      page.locator('input[name="authentication_enabled"]'),
    ).not.toBeChecked();
    await expect(page.locator('input[name="username"]')).toHaveValue("");
    await expect(page.getByText("No password is configured")).toBeVisible();

    // Disabling the relay deletes the complete six-key singleton aggregate.
    await page.locator('input[name="enabled"]').uncheck();
    await saveSmtp(page);
    await reenterAdminSettings(page, "smtp");
    await expect(page.locator('input[name="enabled"]')).not.toBeChecked();
    await expect(page.locator('input[name="host"]')).toHaveValue("");
    await expect(page.locator('input[name="port"]')).toHaveValue("587");
    await expect(page.locator('select[name="tls_mode"]')).toHaveValue(
      "starttls",
    );
    await expect(page.locator('input[name="sender"]')).toHaveValue(
      "Jaunder <noreply@localhost>",
    );
  } finally {
    await disableSmtpIfEnabled(page);
  }
  expect(bootCount(page)).toBe(1);
});

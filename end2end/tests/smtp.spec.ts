import type { Page } from "@playwright/test";
import { expectAccessible } from "./accessibility";
import { reenterAdminSettings } from "./admin-settings";
import { bootCount, trackBoots } from "./bootBudget";
import { test, expect } from "./fixtures";
import { BASE_URL, goto, signInAs, stallServerFn } from "./helpers";
import { openArea, openAreaDestination } from "./settings-areas";
import { SEL } from "./selectors";

const smtpRow = SEL.areaRow("/admin/smtp");
const smtpPage = { url: "/admin/smtp", ready: 'input[name="enabled"]' };
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
  await expect(page.locator("[data-settings-saved]")).toBeVisible();
}

/** Open the SMTP page with the relay disabled, the state the lifecycle starts from. */
async function openSmtpDisabled(page: Page) {
  await openAreaDestination(page, "/settings", smtpPage);
  const enabled = page.locator('input[name="enabled"]');
  await expect(enabled).toBeVisible();
  if (await enabled.isChecked()) {
    await enabled.uncheck();
    await saveSmtp(page);
    // Re-entering remounts the page, so it re-reads the persisted settings.
    await reenterAdminSettings(page, "smtp");
  }
}

test("anonymous SMTP route redirects to Login", async ({ page }) => {
  await goto(page, "/admin/smtp");
  await page.waitForURL(`${BASE_URL}/login?return_to=%2Fadmin%2Fsmtp`);
  await expect(page.locator(SEL.username)).toBeVisible();
  await expect(page.locator('a[href="/admin/smtp"]')).toHaveCount(0);
});

test("member navigation hides SMTP and direct access is denied", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/admin/smtp");
  await expect(page.locator(SEL.error)).toBeVisible();
  await openArea(page, "/settings", SEL.areaRow("/profile"));
  await expect(page.locator(smtpRow)).toHaveCount(0);
});

test("operator manages the complete SMTP relay lifecycle in one document", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  trackBoots(page);
  await goto(page, "/admin/site");

  await openSmtpDisabled(page);

  const enabled = page.locator('input[name="enabled"]');
  const host = page.locator('input[name="host"]');
  const port = page.locator('input[name="port"]');
  const tlsMode = page.locator('select[name="tls_mode"]');
  const sender = page.locator('input[name="sender"]');
  const authentication = page.locator('input[name="authentication_enabled"]');
  const username = page.locator('input[name="username"]');
  const password = page.locator('input[name="password"]');

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
  await expect(page.locator("form.j-card .error")).toContainText(
    "port must not be zero",
  );
  await expect(page.locator(saveButton)).toBeDisabled();
  await port.fill("2525");
  await tlsMode.selectOption("tls");
  await sender.fill("not-a-mailbox");
  await sender.blur();
  await expect(page.locator("form.j-card .error")).toContainText(
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
  await expect(page.locator("[data-settings-saved]")).toContainText(
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
  await expect(page.locator("[data-settings-saved]")).toBeVisible();
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
  await expect(page.locator('select[name="tls_mode"]')).toHaveValue("starttls");
  await expect(page.locator('input[name="sender"]')).toHaveValue(
    "Jaunder <noreply@localhost>",
  );
  expect(bootCount(page)).toBe(1);
});

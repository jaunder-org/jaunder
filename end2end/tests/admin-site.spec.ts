import { reenterAdminSettings } from "./admin-settings";
import { test, expect } from "./fixtures";
import { goto, signInAs, waitForSelector } from "./helpers";
import { allowSecondBoot } from "./bootBudget";
import { SEL } from "./selectors";
import { seedConfigViaTool } from "./seed";

// M8.5: Site settings admin page allows operators to configure site identity.
test("admin site settings page loads and allows updating title and base_url", async ({
  page,
}) => {
  // Log in as operator user
  await signInAs(page, "testoperator");

  await goto(page, "/admin/site");

  // Wait for the form to be visible
  await waitForSelector(page, "input[name='title']");
  await waitForSelector(page, "input[name='base_url']");

  // The save control is a dispatched button (not a native form submit), so it is
  // located by its text (ADR-0065 direct-bind form, mirroring the profile page).
  const submitButton = page.locator('button:has-text("Save Site Settings")');
  await expect(submitButton).toBeVisible();

  await page.fill('input[name="title"]', "My Test Site");
  await page.fill('input[name="base_url"]', "https://example.com");

  // Submit the form and wait for the success status to confirm the write committed
  await submitButton.click();
  await waitForSelector(page, ".j-settings-saved");

  // Re-enter the page in-app and verify the values are persisted: the remount
  // refetches through site::get, so the form is populated from the server.
  await reenterAdminSettings(page, "site");

  // The title round-trips verbatim; the base URL round-trips in its canonical form
  // (`BaseUrl` adds the root path slash).
  await expect(page.locator('input[name="title"]')).toHaveValue("My Test Site");
  await expect(page.locator('input[name="base_url"]')).toHaveValue(
    "https://example.com/",
  );
});

// #552: media uploads are a separately saved site capability. Toggling it must
// not submit or overwrite the independently persisted site identity.
test.describe("Media upload capability", () => {
  test.afterEach(async () => {
    await seedConfigViaTool("media.uploads_enabled", "true");
  });

  test("toggles independently of site identity", async ({ page }) => {
    await signInAs(page, "testoperator");
    await Promise.all([
      page.waitForResponse(
        (response) =>
          response.url().includes("/api/site/get_media_uploads_enabled") &&
          response.request().method() === "POST",
      ),
      goto(page, "/admin/site"),
    ]);

    await waitForSelector(page, 'input[name="title"]');
    await waitForSelector(page, 'input[name="base_url"]');
    await waitForSelector(page, 'input[name="uploads_enabled"]');

    const title = page.locator('input[name="title"]');
    const baseUrl = page.locator('input[name="base_url"]');
    const uploadsEnabled = page.locator('input[name="uploads_enabled"]');
    const saveUploads = page.locator('button:has-text("Save Media Uploads")');
    const initialTitle = await title.inputValue();
    const initialBaseUrl = await baseUrl.inputValue();

    await expect(uploadsEnabled).toBeChecked();
    await uploadsEnabled.uncheck();
    await Promise.all([
      page.waitForResponse(
        (response) =>
          response.url().includes("/api/site/update_media_uploads_enabled") &&
          response.request().method() === "POST",
      ),
      saveUploads.click(),
    ]);
    await waitForSelector(
      page,
      'p.j-settings-saved:has-text("Media upload settings saved.")',
    );

    await reenterAdminSettings(page, "site");
    await waitForSelector(page, 'input[name="uploads_enabled"]');
    await expect(uploadsEnabled).not.toBeChecked();
    await expect(title).toHaveValue(initialTitle);
    await expect(baseUrl).toHaveValue(initialBaseUrl);

    await uploadsEnabled.check();
    await Promise.all([
      page.waitForResponse(
        (response) =>
          response.url().includes("/api/site/update_media_uploads_enabled") &&
          response.request().method() === "POST",
      ),
      saveUploads.click(),
    ]);
    await waitForSelector(
      page,
      'p.j-settings-saved:has-text("Media upload settings saved.")',
    );

    await reenterAdminSettings(page, "site");
    await waitForSelector(page, 'input[name="uploads_enabled"]');
    await expect(uploadsEnabled).toBeChecked();
    await expect(title).toHaveValue(initialTitle);
    await expect(baseUrl).toHaveValue(initialBaseUrl);
  });
});

// #448: the base URL is a typed `Option<BaseUrl>` wire arg — a valid value
// round-trips in canonical form, clearing it dispatches `None` (omitted on the
// wire, decoded to `None`), and a malformed value shows an inline client-side
// error before submit and disables the save button.
test("site base URL round-trips, clears via omission, and validates inline", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/admin/site");
  await waitForSelector(page, "input[name='base_url']");

  const saveButton = page.locator('button:has-text("Save Site Settings")');

  // Set a valid base URL and save.
  await page.fill('input[name="title"]', "Round Trip Site");
  await page.fill('input[name="base_url"]', "https://roundtrip.example.com");
  await saveButton.click();
  await waitForSelector(page, ".j-settings-saved");

  // Re-enter in-app and confirm it round-trips in canonical form.
  await reenterAdminSettings(page, "site");
  await expect(page.locator('input[name="base_url"]')).toHaveValue(
    "https://roundtrip.example.com/",
  );

  // Clear the base URL and save: the empty optional field dispatches `None`, which
  // is omitted on the wire and decodes to `None` (the clear-to-None path).
  await page.fill('input[name="base_url"]', "");
  await page.locator('button:has-text("Save Site Settings")').click();
  await waitForSelector(page, ".j-settings-saved");

  // Re-enter in-app and confirm the base URL is now empty.
  await reenterAdminSettings(page, "site");
  await expect(page.locator('input[name="base_url"]')).toHaveValue("");

  // A malformed URL shows an inline client-side error (once the field is touched)
  // before any submit, and the save button is disabled while invalid.
  const baseUrl = page.locator('input[name="base_url"]');
  await baseUrl.fill("not a url");
  await baseUrl.blur();
  await expect(page.locator(".j-site-form .error")).toBeVisible();
  await expect(
    page.locator('button:has-text("Save Site Settings")'),
  ).toBeDisabled();
});

// M8.5: Non-operators cannot access the site settings page.
test("non-operator user is denied access to /admin/site", async ({ page }) => {
  // Log in as non-operator user
  await signInAs(page, "testlogin");

  // Try to navigate to site settings page
  await goto(page, "/admin/site");

  // Identity and media capability load through separate operator-gated reads, so
  // a denied member sees one real authorization error for each card.
  const errors = page.locator(SEL.error);
  await expect(errors).toHaveCount(2, { timeout: 5_000 });
  await expect(errors.nth(0)).toContainText("unauthorized");
  await expect(errors.nth(1)).toContainText("unauthorized");
});

// #575: the site base-URL warning banner appears in the authed admin chrome when
// `base_url` is unset and disappears once it is configured. After #326 both banners
// share the `.j-warn-banner` class, and the backup banner is *also* visible for
// operators (backup unconfigured by default) — so the site banner is located by its
// copy text, never by class/role. States are driven explicitly (set → hidden, clear →
// visible via the ADR-0065 clear-to-None path) rather than relying on a seed default.
test("site base URL warning banner shows when unset and hides once configured", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/admin/site");
  await waitForSelector(page, "input[name='base_url']");

  const banner = page.getByText("Site base URL is not configured");
  const saveButton = page.locator('button:has-text("Save Site Settings")');

  // Configure a base URL → banner hidden after reload.
  await page.fill('input[name="title"]', "Banner Site");
  await page.fill('input[name="base_url"]', "https://example.com");
  await saveButton.click();
  await waitForSelector(page, ".j-settings-saved");
  allowSecondBoot(
    page,
    "the warning banner is painted from the boot-time site config, so a fresh load is what proves it hides once configured",
  );
  await goto(page, "/admin/site");
  await expect(banner).toBeHidden();

  // Clear the base URL (dispatches `None`) → banner visible after reload.
  await page.fill('input[name="base_url"]', "");
  await page.locator('button:has-text("Save Site Settings")').click();
  await waitForSelector(page, ".j-settings-saved");
  allowSecondBoot(
    page,
    "the warning banner is painted from the boot-time site config, so a fresh load is what proves it reappears once cleared",
  );
  await goto(page, "/admin/site");
  await expect(banner).toBeVisible();
});

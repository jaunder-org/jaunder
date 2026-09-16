import { reenterAdminSettings } from "./admin-settings";
import { test, expect } from "./fixtures";
import { click, goto, signInAs, waitForSelector } from "./helpers";
import { SEL } from "./selectors";
import { seedConfigViaTool } from "./seed";

// Site Settings persists the complete Local identity through one aggregate save.
test("admin site settings page loads, changes, and clears Local identity", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/admin/site");

  await waitForSelector(page, "input[name='title']");
  await waitForSelector(page, "input[name='tagline']");
  await waitForSelector(page, "input[name='base_url']");

  // The save control dispatches the direct-bound typed aggregate (ADR-0065).
  const submitButton = page.locator('button:has-text("Save Site Settings")');
  await expect(submitButton).toBeVisible();

  await page.fill('input[name="title"]', "My Test Site");
  await page.fill('input[name="tagline"]', "The first Local tagline");
  await page.fill('input[name="base_url"]', "https://example.com");
  await submitButton.click();
  await waitForSelector(page, "[data-settings-saved]");

  // An in-app remount re-reads the persisted aggregate identity.
  await reenterAdminSettings(page, "site");
  await expect(page.locator('input[name="title"]')).toHaveValue("My Test Site");
  await expect(page.locator('input[name="tagline"]')).toHaveValue(
    "The first Local tagline",
  );
  await expect(page.locator('input[name="base_url"]')).toHaveValue(
    "https://example.com/",
  );

  await page.fill('input[name="title"]', "Changed Test Site");
  await page.fill('input[name="tagline"]', "The changed Local tagline");
  await submitButton.click();
  await waitForSelector(page, "[data-settings-saved]");
  await reenterAdminSettings(page, "site");
  await expect(page.locator('input[name="title"]')).toHaveValue(
    "Changed Test Site",
  );
  await expect(page.locator('input[name="tagline"]')).toHaveValue(
    "The changed Local tagline",
  );

  // Local uses the persisted identity after the operator signs out.
  await click(page, "a[href='/logout']");
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator('[data-jaunder-part="site-title"]')).toHaveText(
    "Changed Test Site",
  );
  await expect(page.locator(".j-topbar h1")).toHaveText("Changed Test Site");
  await expect(page.locator(".j-topbar .j-sub")).toHaveText(
    "The changed Local tagline",
  );
});

test("Site Settings clears a persisted Local tagline", async ({ page }) => {
  await signInAs(page, "testoperator");
  await goto(page, "/admin/site");
  await waitForSelector(page, "input[name='tagline']");

  const title = await page.locator('input[name="title"]').inputValue();
  const tagline = page.locator('input[name="tagline"]');
  const saveButton = page.locator('button:has-text("Save Site Settings")');
  await tagline.fill("A tagline to clear");
  await saveButton.click();
  await waitForSelector(page, "[data-settings-saved]");

  await tagline.fill("");
  await saveButton.click();
  await waitForSelector(page, "[data-settings-saved]");
  await reenterAdminSettings(page, "site");
  await expect(page.locator('input[name="tagline"]')).toHaveValue("");

  await click(page, "a[href='/logout']");
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator('[data-jaunder-part="site-title"]')).toHaveText(
    title,
  );
  await expect(page.locator(".j-topbar .j-sub")).toHaveCount(0);
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
      'p[data-settings-saved]:has-text("Media upload settings saved.")',
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
      'p[data-settings-saved]:has-text("Media upload settings saved.")',
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
  await waitForSelector(page, "[data-settings-saved]");

  // Re-enter in-app and confirm it round-trips in canonical form.
  await reenterAdminSettings(page, "site");
  await expect(page.locator('input[name="base_url"]')).toHaveValue(
    "https://roundtrip.example.com/",
  );

  // Clear the base URL and save: the empty optional field dispatches `None`, which
  // is omitted on the wire and decodes to `None` (the clear-to-None path).
  await page.fill('input[name="base_url"]', "");
  await page.locator('button:has-text("Save Site Settings")').click();
  await waitForSelector(page, "[data-settings-saved]");

  // Re-enter in-app and confirm the base URL is now empty.
  await reenterAdminSettings(page, "site");
  await expect(page.locator('input[name="base_url"]')).toHaveValue("");

  // A malformed URL shows an inline client-side error (once the field is touched)
  // before any submit, and the save button is disabled while invalid.
  const baseUrl = page.locator('input[name="base_url"]');
  await baseUrl.fill("not a url");
  await baseUrl.blur();
  await expect(page.locator(".j-card .error")).toBeVisible();
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

// #575: the site base-URL warning is a persisted-condition projection in mounted
// authenticated chrome. Each save waits for its write and a fresh warning read;
// request counts prove site saves do not revalidate the backup warning.
test("site base URL warning banner revalidates in place after relevant settings saves", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/admin/site");
  await waitForSelector(page, "input[name='base_url']");

  const title = page.locator('input[name="title"]');
  const baseUrl = page.locator('input[name="base_url"]');
  const banner = page.getByText("Site base URL is not configured");
  const saveButton = page.locator('button:has-text("Save Site Settings")');
  const initialTitle = await title.inputValue();
  const initialBaseUrl = await baseUrl.inputValue();
  let siteWarningRequests = 0;
  let backupWarningRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/site/is_base_url_warning_visible")) {
      siteWarningRequests += 1;
    }
    if (request.url().includes("/api/backup/is_warning_visible")) {
      backupWarningRequests += 1;
    }
  });

  try {
    // Establish the unresolved predicate through the UI before the first assertion.
    await baseUrl.fill("");
    await Promise.all([
      page.waitForResponse((response) =>
        response.url().includes("/api/site/update_identity"),
      ),
      page.waitForResponse((response) =>
        response.url().includes("/api/site/is_base_url_warning_visible"),
      ),
      saveButton.click(),
    ]);
    await expect(banner).toBeVisible();

    let expectedSiteWarnings = siteWarningRequests;
    const expectedBackupWarnings = backupWarningRequests;
    await title.fill("Banner Site");
    await baseUrl.fill("https://example.com");
    await Promise.all([
      page.waitForResponse((response) =>
        response.url().includes("/api/site/update_identity"),
      ),
      page.waitForResponse((response) =>
        response.url().includes("/api/site/is_base_url_warning_visible"),
      ),
      saveButton.click(),
    ]);
    expectedSiteWarnings += 1;
    expect(siteWarningRequests).toBe(expectedSiteWarnings);
    expect(backupWarningRequests).toBe(expectedBackupWarnings);
    await expect(banner).toBeHidden();

    await baseUrl.fill("");
    await Promise.all([
      page.waitForResponse((response) =>
        response.url().includes("/api/site/update_identity"),
      ),
      page.waitForResponse((response) =>
        response.url().includes("/api/site/is_base_url_warning_visible"),
      ),
      saveButton.click(),
    ]);
    expectedSiteWarnings += 1;
    expect(siteWarningRequests).toBe(expectedSiteWarnings);
    expect(backupWarningRequests).toBe(expectedBackupWarnings);
    await expect(banner).toBeVisible();

    // A title-only save completes a fresh warning read but preserves the unresolved predicate.
    await title.fill("Banner Site Renamed");
    await Promise.all([
      page.waitForResponse((response) =>
        response.url().includes("/api/site/update_identity"),
      ),
      page.waitForResponse((response) =>
        response.url().includes("/api/site/is_base_url_warning_visible"),
      ),
      saveButton.click(),
    ]);
    expectedSiteWarnings += 1;
    expect(siteWarningRequests).toBe(expectedSiteWarnings);
    expect(backupWarningRequests).toBe(expectedBackupWarnings);
    await expect(banner).toBeVisible();
  } finally {
    // Restore this test's initial global configuration through the same persisted path.
    await title.fill(initialTitle);
    await baseUrl.fill(initialBaseUrl);
    await Promise.all([
      page.waitForResponse((response) =>
        response.url().includes("/api/site/update_identity"),
      ),
      page.waitForResponse((response) =>
        response.url().includes("/api/site/is_base_url_warning_visible"),
      ),
      saveButton.click(),
    ]);
  }
});

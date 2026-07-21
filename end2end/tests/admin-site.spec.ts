import { test, expect } from "./fixtures";
import { goto, login, waitForSelector } from "./helpers";
import { SEL } from "./selectors";

// M8.5: Site settings admin page allows operators to configure site identity.
test("admin site settings page loads and allows updating title and base_url", async ({
  page,
}) => {
  // Log in as operator user
  await login(page, "testoperator", "testpassword123");

  // Navigate to site settings page
  await goto(page, "/admin/site");

  // Wait for the form to be visible
  await waitForSelector(page, "input[name='title']");
  await waitForSelector(page, "input[name='base_url']");

  // The save control is a dispatched button (not a native form submit), so it is
  // located by its text (ADR-0065 direct-bind form, mirroring the profile page).
  const submitButton = page.locator('button:has-text("Save Site Settings")');
  await expect(submitButton).toBeVisible();

  // Fill in the form
  await page.fill('input[name="title"]', "My Test Site");
  await page.fill('input[name="base_url"]', "https://example.com");

  // Submit the form and wait for the success status to confirm the write committed
  await submitButton.click();
  await waitForSelector(page, ".j-settings-saved");

  // Reload the page and verify values are persisted
  await goto(page, "/admin/site");

  // The title round-trips verbatim; the base URL round-trips in its canonical form
  // (`AbsoluteUrl` adds the root path slash).
  await expect(page.locator('input[name="title"]')).toHaveValue("My Test Site");
  await expect(page.locator('input[name="base_url"]')).toHaveValue(
    "https://example.com/",
  );
});

// #448: the base URL is a typed `Option<AbsoluteUrl>` wire arg — a valid value
// round-trips in canonical form, clearing it dispatches `None` (omitted on the
// wire, decoded to `None`), and a malformed value shows an inline client-side
// error before submit and disables the save button.
test("site base URL round-trips, clears via omission, and validates inline", async ({
  page,
}) => {
  await login(page, "testoperator", "testpassword123");
  await goto(page, "/admin/site");
  await waitForSelector(page, "input[name='base_url']");

  const saveButton = page.locator('button:has-text("Save Site Settings")');

  // Set a valid base URL and save.
  await page.fill('input[name="title"]', "Round Trip Site");
  await page.fill('input[name="base_url"]', "https://roundtrip.example.com");
  await saveButton.click();
  await waitForSelector(page, ".j-settings-saved");

  // Reload and confirm it round-trips in canonical form.
  await goto(page, "/admin/site");
  await expect(page.locator('input[name="base_url"]')).toHaveValue(
    "https://roundtrip.example.com/",
  );

  // Clear the base URL and save: the empty optional field dispatches `None`, which
  // is omitted on the wire and decodes to `None` (the clear-to-None path).
  await page.fill('input[name="base_url"]', "");
  await page.locator('button:has-text("Save Site Settings")').click();
  await waitForSelector(page, ".j-settings-saved");

  // Reload and confirm the base URL is now empty.
  await goto(page, "/admin/site");
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
  await login(page, "testlogin", "testpassword123");

  // Try to navigate to site settings page
  await goto(page, "/admin/site");

  // The page should show an error or redirect
  // Expect to see an error message or be redirected
  await expect(page.locator(SEL.error)).toBeVisible({ timeout: 5_000 });
});

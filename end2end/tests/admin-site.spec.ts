import { test, expect, hydrationHeavyTimeoutMs } from "./fixtures";
import { goto, login, waitForSelector } from "./helpers";

// M8.5: Site settings admin page allows operators to configure site identity.
test("admin site settings page loads and allows updating title and base_url", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 15_000));

  // Log in as operator user
  await login(page, "testoperator", "testpassword123");

  // Navigate to site settings page
  await goto(page, "/admin/site");

  // Wait for the form to be visible
  await waitForSelector(page, "input[name='title']");
  await waitForSelector(page, "input[name='base_url']");

  // Verify the form is present with the button
  const submitButton = page.locator('button:has-text("Save Site Settings")');
  await expect(submitButton).toBeVisible();

  // Fill in the form
  await page.fill('input[name="title"]', "My Test Site");
  await page.fill('input[name="base_url"]', "https://example.com");

  // Submit the form and wait for the success status to confirm the write committed
  await page.click('button[type="submit"]');
  await waitForSelector(page, ".j-settings-saved");

  // Reload the page and verify values are persisted
  await goto(page, "/admin/site");

  // Check that the values were saved
  const titleInput = page.locator('input[name="title"]');
  const baseUrlInput = page.locator('input[name="base_url"]');

  await expect(titleInput).toHaveValue("My Test Site");
  await expect(baseUrlInput).toHaveValue("https://example.com");
});

// M8.5: Non-operators cannot access the site settings page.
test("non-operator user is denied access to /admin/site", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 10_000));

  // Log in as non-operator user
  await login(page, "testlogin", "testpassword123");

  // Try to navigate to site settings page
  await goto(page, "/admin/site");

  // The page should show an error or redirect
  // Expect to see an error message or be redirected
  await expect(page.locator(".error")).toBeVisible({ timeout: 5_000 });
});

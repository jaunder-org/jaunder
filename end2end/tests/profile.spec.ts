import { test, expect } from "./fixtures";
import { goto } from "./helpers";
import { SEL } from "./selectors";

// The profile "Update Profile" control is a plain button that dispatches the
// typed UpdateProfile server fn (ADR-0065), not an <ActionForm> submit — select
// it by its label.
const UPDATE_BUTTON = 'button:has-text("Update Profile")';
const DISPLAY_NAME = 'input[name="display_name"]';

// #401: a valid display name entered on the profile page persists across a reload.
test("profile update persists a valid display name", async ({
  registeredPage: page,
}) => {
  await goto(page, "/profile");

  await page.fill(DISPLAY_NAME, "Ada Lovelace");

  const updated = page.waitForResponse((response) =>
    response.url().includes("update_profile"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  // A fresh load reads the persisted value back through get_profile.
  await goto(page, "/profile");
  await expect(page.locator(DISPLAY_NAME)).toHaveValue("Ada Lovelace");
});

// #401: an over-long entry (> 255 chars) is rejected client-side by the shared
// DisplayName FromStr — the newtype's own message shows inline once the field is
// touched, and submit is disabled (ADR-0065 disable-until-valid).
test("over-long display name shows an inline error and gates submit", async ({
  registeredPage: page,
}) => {
  await goto(page, "/profile");

  const input = page.locator(DISPLAY_NAME);
  await input.fill("a".repeat(256));
  await input.blur();

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(UPDATE_BUTTON)).toBeDisabled();
});

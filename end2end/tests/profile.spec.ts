import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { failServerFn, goto, signInAsNewUser } from "./helpers";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";

// The profile "Update Profile" control is a plain button that dispatches the
// typed UpdateProfile server fn (ADR-0065), not an <ActionForm> submit — select
// it by its label.
const UPDATE_BUTTON = 'button:has-text("Update Profile")';
const DISPLAY_NAME = 'input[name="display_name"]';
const BIO = 'textarea[name="bio"]';

// #21: Settings is the authenticated user's in-app route to the profile page.
// The link assertion catches a disabled or misdirected sidebar item; the router
// transition exercises the route a user actually takes without a second boot.
test("Settings navigates to profile", async ({ registeredPage }) => {
  const page = await registeredPage("/app");
  const settings = page.getByRole("link", { name: "Settings" });

  await expect(settings).toHaveAttribute("href", "/profile");
  await navigateInApp(page, () => settings.click(), {
    url: "/profile",
    ready: UPDATE_BUTTON,
  });
  await expect(page.locator(UPDATE_BUTTON)).toBeVisible();
});

const APP_LINK = 'a[href="/app"]';
const SETTINGS_LINK = 'a[href="/profile"]';

async function reenterProfile(page: Page): Promise<void> {
  await navigateInApp(page, () => page.click(APP_LINK), {
    url: "/app",
    ready: SEL.postBody,
  });
  await navigateInApp(page, () => page.click(SETTINGS_LINK), {
    url: "/profile",
    ready: UPDATE_BUTTON,
  });
}

// #401: a valid display name entered on the profile page persists after re-entry.
test("profile update persists a valid display name", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/profile");

  await page.fill(DISPLAY_NAME, "Ada Lovelace");

  const updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  // Re-enter through the Settings affordance so profile::get reads the persisted value.
  await reenterProfile(page);
  await expect(page.locator(DISPLAY_NAME)).toHaveValue("Ada Lovelace");
});

// #401/#545: over-long Profile fields are rejected client-side by the shared
// DisplayName/Bio FromStr. The newtype's own message shows inline once the field
// is touched, and submit is disabled (ADR-0065 disable-until-valid).
const PROFILE_VALIDATION_CASES = [
  {
    field: "display name",
    selector: DISPLAY_NAME,
    invalidValue: "a".repeat(256),
  },
  { field: "bio", selector: BIO, invalidValue: "a".repeat(1001) },
] as const;

for (const validationCase of PROFILE_VALIDATION_CASES) {
  test(`over-long ${validationCase.field} shows an inline error and gates submit`, async ({
    registeredPage,
  }) => {
    const page = await registeredPage("/profile");

    const input = page.locator(validationCase.selector);
    await input.fill(validationCase.invalidValue);
    await input.blur();

    await expect(page.locator(SEL.error)).toBeVisible();
    await expect(page.locator(UPDATE_BUTTON)).toBeDisabled();
  });
}

// #401: clearing the box removes the display name end-to-end. Under the typed
// Option<DisplayName> wire arg an empty value is *omitted* (dispatched as None),
// not sent as an empty string that would fail to decode — so emptying the field
// and submitting must persist as cleared, and submit stays enabled (empty is a
// valid optional value). This is the real-browser form of the former
// "empty fields set to none" server test.
test("clearing the display name persists as empty", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/profile");

  await page.fill(DISPLAY_NAME, "Temp Name");
  let updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  await reenterProfile(page);
  await expect(page.locator(DISPLAY_NAME)).toHaveValue("Temp Name");

  // Empty the field (valid for an optional field ⇒ submit stays enabled) and save.
  await page.fill(DISPLAY_NAME, "");
  updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  await reenterProfile(page);
  await expect(page.locator(DISPLAY_NAME)).toHaveValue("");
});

// #545: a valid bio entered on the profile page persists after re-entry through
// the typed Option<Bio> wire arg round-trip in profile::update/profile::get.
test("profile update persists a valid bio", async ({ registeredPage }) => {
  const page = await registeredPage("/profile");

  await page.fill(BIO, "Mathematician and first programmer.");

  const updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  await reenterProfile(page);
  await expect(page.locator(BIO)).toHaveValue(
    "Mathematician and first programmer.",
  );
});

// #498/#324: the "Default post format" control is an ADR-0065 direct-bind — a
// plain <select> bound to a signal whose value a "Save" button dispatches as the
// typed PostFormat wire arg over server_fn's Url codec (serde_qs), not an
// <ActionForm> submit. Selecting a format, saving, and re-entering must round-trip
// the chosen value through set_default_post_format/get_default_post_format —
// proving the typed arg encodes and decodes. Two flips confirm it persists the
// *selected* value, not a constant.
const FORMAT_SELECT = "select#default-post-format";
const FORMAT_SAVE = 'button:has-text("Save")';

test("default post format round-trips through the typed dispatch", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/profile");

  const saveAndReenter = async (value: string) => {
    await page.selectOption(FORMAT_SELECT, value);
    const saved = page.waitForResponse((response) =>
      response.url().includes("set_default_post_format"),
    );
    await page.click(FORMAT_SAVE);
    expect((await saved).ok()).toBe(true);
    await reenterProfile(page);
    await expect(page.locator(FORMAT_SELECT)).toHaveValue(value);
  };

  await saveAndReenter("org");
  await saveAndReenter("markdown");
});

// #58: the default-format request is authoritative. A transport failure must
// not seed the direct-bound control with Markdown and thereby make Save capable
// of overwriting the persisted preference with a value the server never returned.
test("failed default post format load shows an error and gates Save", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await failServerFn(page, "profile/get_default_post_format");
  await goto(page, "/profile");

  const control = page.locator(".j-card", {
    hasText: "Default Post Format",
  });
  await expect(control.locator("p.error")).toHaveText(
    "Could not load the default post format.",
  );
  await expect(control.locator(FORMAT_SELECT)).toHaveCount(0);
  await expect(control.locator(FORMAT_SAVE)).toBeDisabled();
});

// #545: clearing the box removes the bio end-to-end. Under the typed Option<Bio>
// wire arg an empty value is *omitted* (dispatched as None), not sent as an empty
// string that would fail to decode — so emptying the field and submitting must
// persist as cleared, and submit stays enabled (empty is a valid optional value).
test("clearing the bio persists as empty", async ({ registeredPage }) => {
  const page = await registeredPage("/profile");

  await page.fill(BIO, "Temporary bio");
  let updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  await reenterProfile(page);
  await expect(page.locator(BIO)).toHaveValue("Temporary bio");

  // Empty the field (valid for an optional field ⇒ submit stays enabled) and save.
  await page.fill(BIO, "");
  updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  await reenterProfile(page);
  await expect(page.locator(BIO)).toHaveValue("");
});

import { test, expect } from "./fixtures";
import { failServerFn, goto, signInAsNewUser } from "./helpers";
import { allowSecondBoot } from "./bootBudget";
import { SEL } from "./selectors";

// The profile "Update Profile" control is a plain button that dispatches the
// typed UpdateProfile server fn (ADR-0065), not an <ActionForm> submit — select
// it by its label.
const UPDATE_BUTTON = 'button:has-text("Update Profile")';
const DISPLAY_NAME = 'input[name="display_name"]';
const BIO = 'textarea[name="bio"]';

// #401: a valid display name entered on the profile page persists across a reload.
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

  // Re-read the persisted value from the server. That takes a document load here:
  // `/profile` is a route with no in-app entry point (checked across `web/src` —
  // it is in neither `NAV_ITEMS` nor the sidebar footer), and #896's rule is that
  // a test never invents an affordance the app does not have.
  allowSecondBoot(
    page,
    "nothing in the app links to /profile — no sidebar nav item, no footer avatar link — so there is no in-app move that re-enters the route, and a document load is the only way to remount the page and re-read the value through profile::get",
  );
  await goto(page, "/profile");
  await expect(page.locator(DISPLAY_NAME)).toHaveValue("Ada Lovelace");
});

// #401: an over-long entry (> 255 chars) is rejected client-side by the shared
// DisplayName FromStr — the newtype's own message shows inline once the field is
// touched, and submit is disabled (ADR-0065 disable-until-valid).
test("over-long display name shows an inline error and gates submit", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/profile");

  const input = page.locator(DISPLAY_NAME);
  await input.fill("a".repeat(256));
  await input.blur();

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(UPDATE_BUTTON)).toBeDisabled();
});

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

  allowSecondBoot(
    page,
    "nothing in the app links to /profile — no sidebar nav item, no footer avatar link — so there is no in-app move that re-enters the route, and a document load is the only way to remount the page and re-read the value through profile::get",
  );
  await goto(page, "/profile");
  await expect(page.locator(DISPLAY_NAME)).toHaveValue("Temp Name");

  // Empty the field (valid for an optional field ⇒ submit stays enabled) and save.
  await page.fill(DISPLAY_NAME, "");
  updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  allowSecondBoot(
    page,
    "the None round-trip needs the cleared display name read back from the server, and with no in-app link to /profile a document load is the only way to re-enter the route",
  );
  await goto(page, "/profile");
  await expect(page.locator(DISPLAY_NAME)).toHaveValue("");
});

// #545: a valid bio entered on the profile page persists across a reload — the
// typed Option<Bio> wire arg round-trips through profile::update/profile::get.
test("profile update persists a valid bio", async ({ registeredPage }) => {
  const page = await registeredPage("/profile");

  await page.fill(BIO, "Mathematician and first programmer.");

  const updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  allowSecondBoot(
    page,
    "nothing in the app links to /profile — no sidebar nav item, no footer avatar link — so there is no in-app move that re-enters the route, and a document load is the only way to remount the page and re-read the value through profile::get",
  );
  await goto(page, "/profile");
  await expect(page.locator(BIO)).toHaveValue(
    "Mathematician and first programmer.",
  );
});

// #545: an over-long bio (> MAX_BIO_CHARS = 1000) is rejected client-side by the
// shared Bio FromStr — the newtype's own message shows inline once touched, and
// submit is disabled (ADR-0065 disable-until-valid, gated on bio validity too).
test("over-long bio shows an inline error and gates submit", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/profile");

  const input = page.locator(BIO);
  await input.fill("a".repeat(1001));
  await input.blur();

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(UPDATE_BUTTON)).toBeDisabled();
});

// #498/#324: the "Default post format" control is an ADR-0065 direct-bind — a
// plain <select> bound to a signal whose value a "Save" button dispatches as the
// typed PostFormat wire arg over server_fn's Url codec (serde_qs), not an
// <ActionForm> submit. Selecting a format, saving, and reloading must round-trip
// the chosen value through set_default_post_format/get_default_post_format —
// proving the typed arg encodes and decodes. Two flips confirm it persists the
// *selected* value, not a constant.
const FORMAT_SELECT = "select#default-post-format";
const FORMAT_SAVE = 'button:has-text("Save")';

test("default post format round-trips through the typed dispatch", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/profile");

  // Each flip's reload is a declared second boot, and the two flips are declared
  // for different reasons, so the reason is a parameter rather than a constant.
  const saveAndReload = async (value: string, reason: string) => {
    await page.selectOption(FORMAT_SELECT, value);
    const saved = page.waitForResponse((response) =>
      response.url().includes("set_default_post_format"),
    );
    await page.click(FORMAT_SAVE);
    expect((await saved).ok()).toBe(true);
    allowSecondBoot(page, reason);
    await goto(page, "/profile");
    await expect(page.locator(FORMAT_SELECT)).toHaveValue(value);
  };

  await saveAndReload(
    "org",
    "no in-app link re-enters /profile, so a document load is the only way to read the saved default post format back through get_default_post_format",
  );
  await saveAndReload(
    "markdown",
    "the second flip proves the persisted value is the selected one and not a constant, and it needs the same load for the same reason: nothing in the app navigates to /profile",
  );
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

  allowSecondBoot(
    page,
    "nothing in the app links to /profile — no sidebar nav item, no footer avatar link — so there is no in-app move that re-enters the route, and a document load is the only way to remount the page and re-read the value through profile::get",
  );
  await goto(page, "/profile");
  await expect(page.locator(BIO)).toHaveValue("Temporary bio");

  // Empty the field (valid for an optional field ⇒ submit stays enabled) and save.
  await page.fill(BIO, "");
  updated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await updated).ok()).toBe(true);

  allowSecondBoot(
    page,
    "the None round-trip needs the cleared bio read back from the server, and with no in-app link to /profile a document load is the only way to re-enter the route",
  );
  await goto(page, "/profile");
  await expect(page.locator(BIO)).toHaveValue("");
});

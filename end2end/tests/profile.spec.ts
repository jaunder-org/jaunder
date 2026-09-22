import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import {
  BASE_URL,
  failServerFn,
  goto,
  signInAsNewUser,
  waitForMount,
} from "./helpers";
import { navigateInApp } from "./navigate";
import { allowSecondBoot } from "./bootBudget";
import { fetchFeedContaining } from "./feeds";
import { createPostViaApi } from "./posts";
import { applySeededSession, seedUserViaTool } from "./seed";
import { SEL } from "./selectors";

// The profile "Update Profile" control is a plain button that dispatches the
// typed UpdateProfile server fn (ADR-0065), not an <ActionForm> submit — select
// it by its label.
const UPDATE_BUTTON = 'button:has-text("Update Profile")';
const DISPLAY_NAME = 'input[name="display_name"]';
const BIO = 'textarea[name="bio"]';
const CONTENT_LICENSE = "select#content-license";
const CONTENT_LICENSE_SAVE = 'button:has-text("Save Content License")';

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

test("Profile presents Username as a read-only field", async ({ page }) => {
  const session = await seedUserViaTool(
    "profile-readonly",
    "profile-password123",
  );
  await applySeededSession(page.context(), session);
  await goto(page, "/profile");

  const username = page.getByLabel("Username");
  await expect(username).toHaveValue(session.username);
  await expect(username).toBeEnabled();
  await expect(username).not.toBeEditable();
  await expect(username).toHaveAttribute("readonly", "");
  await expect(page.getByText("Your display name and bio.")).toHaveCount(0);
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
// #1611: Content License is a current, publication-wide setting. This drives
// both server functions through the real selector, proves reload persistence,
// and observes its retroactive public HTML and Atom Syndication Feed effect on
// a Post that existed before the setting changed.
test("Content License persists and updates existing public Post declarations", async ({
  page,
}) => {
  const username = await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: "# Existing Content Rights Post\n\nThe authored body stays unchanged.",
  });
  const year = new Date().getUTCFullYear();
  const declaration = `© ${year} Ada Current`;

  await goto(page, "/profile");
  await expect(page.locator(CONTENT_LICENSE)).toHaveValue(
    "all-rights-reserved",
  );
  await expect(
    page.getByText("This choice applies retroactively to all of your Posts."),
  ).toBeVisible();

  // A current Display Name is part of every Copyright Declaration. Profile
  // field persistence itself has focused coverage above; this makes its public
  // projection observable without duplicating that lower-level round trip.
  await page.fill(DISPLAY_NAME, "Ada Current");
  const profileUpdated = page.waitForResponse((response) =>
    response.url().includes("profile/update"),
  );
  await page.click(UPDATE_BUTTON);
  expect((await profileUpdated).ok()).toBe(true);

  const defaultHtml = await page.request.get(`${BASE_URL}${post.permalink}`);
  expect(defaultHtml.ok(), "default public permalink").toBeTruthy();
  const defaultBody = await defaultHtml.text();
  expect(defaultBody).toContain(`${declaration} · All Rights Reserved`);
  expect(defaultBody).not.toContain('rel="license"');

  const defaultFeed = await fetchFeedContaining(
    page.request,
    `${BASE_URL}/~${username}/feed.atom`,
    `<rights>${declaration} · All Rights Reserved</rights>`,
  );
  expect(defaultFeed.body).toContain(
    `<rights>${declaration} · All Rights Reserved</rights>`,
  );
  expect(defaultFeed.body).not.toContain('rel="license"');

  await page.selectOption(CONTENT_LICENSE, "CC-BY-4.0");
  const licenseSaved = page.waitForResponse((response) =>
    response.url().includes("profile/set_content_license"),
  );
  await page.click(CONTENT_LICENSE_SAVE);
  expect((await licenseSaved).ok()).toBe(true);

  // A real document reload makes get_content_license read storage again rather
  // than retaining the selector's client signal.
  allowSecondBoot(
    page,
    "the Content License setting must survive a document reload, not merely retain its in-memory selector value",
  );
  await page.reload({ waitUntil: "domcontentloaded" });
  await waitForMount(page);
  await expect(page.locator(CONTENT_LICENSE)).toHaveValue("CC-BY-4.0");

  const ccHtml = await page.request.get(`${BASE_URL}${post.permalink}`);
  expect(ccHtml.ok(), "Creative Commons public permalink").toBeTruthy();
  expect(await ccHtml.text()).toContain(
    `${declaration} · <a href="https://creativecommons.org/licenses/by/4.0/" rel="license">CC BY 4.0</a>`,
  );

  const ccFeed = await fetchFeedContaining(
    page.request,
    `${BASE_URL}/~${username}/feed.atom`,
    "CC BY 4.0",
  );
  expect(ccFeed.body).toContain(`<rights>${declaration} · CC BY 4.0</rights>`);
  expect(ccFeed.body).toContain(
    '<link href="https://creativecommons.org/licenses/by/4.0/" rel="license" type="text/html"',
  );
});

// #1611: direct-bind mutations must surface their failed acknowledgement just
// like the adjacent profile controls rather than silently leaving the selector
// in a misleading client-only state.
test("failed Content License save shows mutation feedback", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/profile");
  await expect(page.locator(CONTENT_LICENSE)).toHaveValue(
    "all-rights-reserved",
  );
  await failServerFn(page, "profile/set_content_license");

  await page.selectOption(CONTENT_LICENSE, "CC-BY-4.0");
  await page.click(CONTENT_LICENSE_SAVE);
  await expect(
    page.locator(".j-card", { hasText: "Content License" }).locator("p.error"),
  ).toBeVisible();
});

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

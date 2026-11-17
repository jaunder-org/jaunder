/**
 * #1733 — the Settings and Account corner controls and the Settings and
 * Account pages they open.
 *
 * Configuration left the sidebar: every page header ends, for a signed-in
 * viewer, with a Settings and an Account control, each opening a page that
 * links to the existing configuration pages. Rows follow the viewer rules
 * (operators alone see Site administration); a settings area's sub-pages link
 * back to it and keep its control marked current.
 */
import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import {
  BASE_URL,
  click,
  failServerFn,
  fillLoginForm,
  goto,
  signInAs,
  signInAsNewUser,
  waitForSelector,
} from "./helpers";
import { openArea, signOutViaAccount } from "./settings-areas";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";

const AREA_CONTROLS = ".j-topbar .j-area-controls";
const SETTINGS_CONTROL = SEL.areaControl("/settings");
const BACK_LINK = ".j-topbar .j-topbar-back";

/** Both controls are present, last in the header's right slot, and — where the
 *  page has a timeline sort control — after it. */
async function expectAreaControls(page: Page, afterSort: boolean) {
  await expect(page.locator(AREA_CONTROLS)).toBeVisible();
  await expect(page.locator(SETTINGS_CONTROL)).toHaveAttribute(
    "aria-label",
    "Settings",
  );
  await expect(page.locator(SEL.accountControl)).toHaveAttribute(
    "aria-label",
    "Account",
  );
  const right = page.locator(".j-topbar .j-topbar-right");
  expect(
    await right.evaluate((slot) =>
      slot.lastElementChild?.classList.contains("j-area-controls"),
    ),
  ).toBe(true);
  if (afterSort) {
    const sort = right.locator(".j-timeline-order");
    await expect(sort).toBeVisible();
    const sortBox = (await sort.boundingBox())!;
    const controlsBox = (await page.locator(AREA_CONTROLS).boundingBox())!;
    expect(controlsBox.x).toBeGreaterThanOrEqual(sortBox.x + sortBox.width);
  }
}

test("signed-in member sees the controls after the sort button on Home", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/app");
  await expectAreaControls(page, true);
  await expect(page.locator(SETTINGS_CONTROL)).not.toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("signed-in member sees the controls after the sort button on a public timeline", async ({
  page,
}) => {
  const username = await signInAsNewUser(page);
  await goto(page, `/~${username}`);
  await expectAreaControls(page, true);
});

test("signed-in member sees the controls on a private page", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/drafts");
  await expect(page.locator(SEL.topbarHeading)).toHaveText("Drafts");
  await expectAreaControls(page, false);
});

test("anonymous viewer sees no Settings or Account controls", async ({
  page,
}) => {
  // `goto` waits for the CSR mount, so an absent control is a real absence
  // rather than an unmounted tree.
  await goto(page, "/");
  await expect(page.locator(".j-topbar .j-timeline-order")).toBeVisible();
  await expect(page.locator(".j-area-controls")).toHaveCount(0);
  await expect(page.locator(".j-area-control")).toHaveCount(0);
});

test("in-app login shows the controls without a reload", async ({
  page,
  user,
  firstNav,
}) => {
  await goto(page, "/login", { timeout: firstNav });
  await expect(page.locator(".j-area-controls")).toHaveCount(0);
  await page.evaluate(() => {
    (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload =
      true;
  });

  await fillLoginForm(page, user.username, user.password);
  await waitForSelector(page, SEL.accountControl);
  await expect(page).toHaveURL(`${BASE_URL}/app`);
  await expectAreaControls(page, true);
  expect(
    await page.evaluate(
      () =>
        (window as Window & { __jaunderNoReload?: boolean })
          .__jaunderNoReload === true,
    ),
  ).toBe(true);
});

test("member Settings page has no Site administration group", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/app");
  await openArea(page, "/settings", SEL.areaRow("/profile"));

  await expect(page.locator(SETTINGS_CONTROL)).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(SEL.accountControl)).not.toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(BACK_LINK)).toHaveCount(0);
  await expect(page.locator(SEL.areaRow("/themes"))).toBeVisible();
  await expect(
    page.locator(".j-area-group h2", { hasText: "Site administration" }),
  ).toHaveCount(0);
  await expect(page.locator('.j-area-row[href^="/admin/"]')).toHaveCount(0);
});

test("Settings page keeps its rows when the Registration Policy fails to load", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/app");
  await failServerFn(page, "registration/get_policy");
  await openArea(page, "/settings", SEL.areaRow("/profile"));

  await expect(page.locator(".j-settings p.error")).toContainText(
    "Invitation availability could not be loaded",
  );
  await expect(page.locator(SEL.areaRow("/themes"))).toBeVisible();
  await expect(page.locator(SEL.areaRow("/invites"))).toHaveCount(0);
});

const ADMIN_DESTINATIONS = [
  { url: "/admin/backups", ready: 'input[name="destination_path"]' },
  { url: "/admin/site", ready: 'input[name="title"]' },
  { url: "/admin/smtp", ready: 'input[name="host"]' },
  { url: "/admin/websub", ready: 'input[name="hub_url"]' },
] as const;

test("operator Settings page lists every admin page and each row reaches it", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/app");
  await openArea(page, "/settings", SEL.areaRow("/admin/backups"));

  const adminGroup = page.locator(".j-area-group", {
    has: page.locator("h2", { hasText: "Site administration" }),
  });
  await expect(adminGroup.locator(".j-area-row")).toHaveCount(
    ADMIN_DESTINATIONS.length,
  );
  for (const destination of ADMIN_DESTINATIONS) {
    await expect(
      adminGroup.locator(SEL.areaRow(destination.url)),
    ).toBeVisible();
  }

  for (const [index, destination] of ADMIN_DESTINATIONS.entries()) {
    if (index > 0) {
      await openArea(page, "/settings", SEL.areaRow(destination.url));
    }
    await navigateInApp(
      page,
      () => click(page, SEL.areaRow(destination.url)),
      destination,
    );
    await expect(page.locator(BACK_LINK)).toHaveText("← Settings");
    await expect(page.locator(SETTINGS_CONTROL)).toHaveAttribute(
      "aria-current",
      "page",
    );
  }
});

test("Account → Passkeys shows the back link and keeps Account current", async ({
  page,
}) => {
  const username = await signInAsNewUser(page);
  await goto(page, "/app");
  await openArea(page, "/account", SEL.areaRow("/passkeys"));

  await expect(page.locator(".j-area-identity")).toContainText(username);
  await expect(page.locator(SEL.accountControl)).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(SETTINGS_CONTROL)).not.toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(page.locator(BACK_LINK)).toHaveCount(0);

  await navigateInApp(page, () => click(page, SEL.areaRow("/passkeys")), {
    url: "/passkeys",
    ready: '[data-test="passkeys-page"]',
  });
  await expect(page.locator(BACK_LINK)).toHaveText("← Account");
  await expect(page.locator(BACK_LINK)).toHaveAttribute("href", "/account");
  await expect(page.locator(SEL.accountControl)).toHaveAttribute(
    "aria-current",
    "page",
  );

  await navigateInApp(page, () => click(page, BACK_LINK), {
    url: "/account",
    ready: SEL.areaRow("/passkeys"),
  });
});

test("Account → Sign out signs out and the controls disappear", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/app");
  await expect(page.locator(AREA_CONTROLS)).toBeVisible();

  await signOutViaAccount(page);
  await page.waitForURL(`${BASE_URL}/`);
  await expect(page.locator(".j-nav a[href='/']")).toHaveText("Local");
  await expect(page.locator(".j-area-controls")).toHaveCount(0);
  await expect(page.locator(".j-area-control")).toHaveCount(0);
});

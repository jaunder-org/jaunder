import { test, expect } from "./fixtures";
import {
  click,
  goto,
  signInAs,
  signInAsNewUser,
  waitForSelector,
} from "./helpers";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";

const REVOKED_SESSION_LABEL = "Revoked session e2e";

// Browser-flow coverage for `sessions::revoke` (#707): server tests already pin
// ownership and token-death semantics, but the coverage gate needs a real
// traced browser request from the Sessions UI.
test("sessions page revokes another browser session", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  const otherContext = await tracedContext();

  try {
    const otherPage = await otherContext.newPage();
    await signInAs(otherPage, username, REVOKED_SESSION_LABEL);
    await goto(otherPage, "/");
    await waitForSelector(otherPage, SEL.logoutLink);

    await goto(page, "/sessions");

    const currentRow = page.locator("li", { hasText: "(current)" });
    const revokedRow = page.locator("li", { hasText: REVOKED_SESSION_LABEL });
    await expect(currentRow).toBeVisible();
    await expect(revokedRow).toBeVisible();

    await click(
      page,
      `li:has-text("${REVOKED_SESSION_LABEL}") button:has-text("Revoke")`,
    );

    await expect(revokedRow).toHaveCount(0);
    await expect(currentRow).toBeVisible();

    await navigateInApp(
      otherPage,
      () => click(otherPage, '.j-nav a[href="/app"]'),
      { url: "/login", ready: SEL.username },
    );
    await expect(otherPage.locator(SEL.logoutLink)).toHaveCount(0);
  } finally {
    await otherContext.close();
  }
});

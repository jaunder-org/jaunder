import {
  test,
  expect,
  setTestBudget,
  slowBrowserFirstNavigationTimeoutMs,
} from "./fixtures";
import { goto, login, click, waitForSelector } from "./helpers";
import { SEL } from "./selectors";
import { extractInviteCode } from "./mail";
import { seedConfigViaTool } from "./seed";

// #433: the invitation round trip. These tests flip `site.registration_policy`
// to `invite_only` — a global site-config singleton — so this spec runs in the
// serial `*-admin` Playwright project (after the parallel main project), exactly
// like admin-site.spec, and never overlaps specs that register users under the
// seeded `open` policy. The default is restored in afterAll.
test.afterAll(() => {
  // Restore both globals this spec mutates so a later serial `-admin` spec can't
  // inherit them (Test A sets base_url; both tests set the policy).
  seedConfigViaTool("site.registration_policy", "open");
  seedConfigViaTool("site.base_url", "");
});

// Test A — the main flow: an operator emails an invite, and the invitee follows
// the link and registers with no manual code entry (the register page reads the
// code from the URL and submits it as a hidden field).
test("invite link registration completes end-to-end", async ({
  page,
  browser,
  user,
  mailbox,
}) => {
  // Full round trip: operator login + email delivery + a cold fresh-context
  // registration all in one test.
  setTestBudget(45_000);

  // Establish invite-only and a base URL so create_invite can build the link
  // (`{base_url}/register?invite_code=<code>`); it errors without a base URL.
  seedConfigViaTool("site.registration_policy", "invite_only");
  seedConfigViaTool("site.base_url", "https://example.com");

  // The operator sends an invite to this test's mailbox recipient via the
  // /invites UI (shows a "Page not found." fallback unless invite_only, which
  // we just set).
  await login(page, "testoperator", "testpassword123");
  await goto(page, "/invites");
  await page.fill('input[name="recipient_email"]', user.email);
  await page.fill('input[name="expires_in_hours"]', "168");
  await click(page, SEL.submit);
  await waitForSelector(page, 'p:has-text("Invitation emailed to")');

  // Read the invitation email and pull the code out of the link.
  const email = await mailbox.waitForNewEmail();
  const code = extractInviteCode(email);

  // A fresh, logged-out visitor follows the invite link and registers. No code
  // is typed — the register page carries it from the URL as a hidden field.
  const context = await browser.newContext();
  try {
    const invitee = await context.newPage();
    const firstNav = slowBrowserFirstNavigationTimeoutMs(test.info(), 15_000);
    const username = `invitee${Date.now()}${Math.random()
      .toString(36)
      .slice(2, 8)}`;
    await goto(invitee, `/register?invite_code=${code}`, { timeout: firstNav });
    await invitee.fill(SEL.username, username);
    await invitee.fill(SEL.password, "testpassword123");
    await click(invitee, SEL.submit);

    // Race the success marker against an explicit error so a redemption failure
    // fails fast with its message rather than burning the whole timeout.
    const outcome = await Promise.race([
      invitee
        .waitForSelector(SEL.logoutLink, { timeout: 10_000 })
        .then(() => "ok"),
      invitee
        .waitForSelector(SEL.error, { timeout: 10_000 })
        .then(() => "error"),
    ]);
    if (outcome === "error") {
      const errorText = (
        await invitee.locator(SEL.error).first().textContent()
      )?.trim();
      throw new Error(
        `invite registration failed: ${errorText ?? "unknown error"}`,
      );
    }
  } finally {
    await context.close();
  }
});

// Test B — no-code guidance: in invite_only mode, visiting /register with no
// invite_code shows the guidance text and renders no register submit button.
test("invite-only /register with no code shows guidance and no submit button", async ({
  page,
}) => {
  seedConfigViaTool("site.registration_policy", "invite_only");
  const firstNav = slowBrowserFirstNavigationTimeoutMs(test.info(), 15_000);

  await goto(page, "/register", { timeout: firstNav });

  await expect(
    page.locator('p:has-text("You need an invitation link to register")'),
  ).toBeVisible({ timeout: 10_000 });
  // The guidance branch replaces the whole form — no register submit button.
  await expect(
    page.locator('.j-page-narrow button[type="submit"]'),
  ).toHaveCount(0);
});

// Test C — policy guard: on a non-invite-only site the authed /invites page
// renders the "Page not found." fallback and no create form. Locks the
// client-side policy-gating (#320 removed the dead SSR set_status 404). Self-sets
// `open`, so placement is order-independent; the file's afterAll restores `open`.
test("invites page shows not-found fallback when not invite-only", async ({
  page,
}) => {
  seedConfigViaTool("site.registration_policy", "open");
  const firstNav = slowBrowserFirstNavigationTimeoutMs(test.info(), 15_000);

  await login(page, "testoperator", "testpassword123");
  await goto(page, "/invites", { timeout: firstNav });

  await expect(page.locator('p:has-text("Page not found.")')).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.locator('input[name="recipient_email"]')).toHaveCount(0);
});

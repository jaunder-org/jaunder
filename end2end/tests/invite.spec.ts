import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import {
  generateUsername,
  goto,
  signInAs,
  click,
  waitForSelector,
  failServerFn,
  stallServerFn,
} from "./helpers";
import { SEL } from "./selectors";
import { extractInviteCode } from "./mail";
import { seedConfigViaTool } from "./seed";

// #433: the invitation round trip and policy projection. These tests flip
// `site.registration_policy` — a global site-config singleton — so this spec runs in the
// serial `*-admin` Playwright project (after the parallel main project), exactly like
// admin-site.spec, and never overlaps specs that register users under the seeded `open`
// policy. The default is restored in afterAll.
test.afterAll(async () => {
  // Restore the globals this spec mutates so a later serial `-admin` spec cannot
  // inherit them (Test A sets base_url; every case sets the policy).
  await seedConfigViaTool("site.registration_policy", "open");
  await seedConfigViaTool("site.base_url", "");
});

// Test A — the main flow: an operator emails an invite, and the invitee follows
// the link and registers with no manual code entry (the register page reads the
// code from the URL and submits it as a hidden field).
test("invite link registration completes end-to-end", async ({
  page,
  tracedContext,
  user,
  mailbox,
}) => {
  // Establish operator-issued invitations and a base URL so invites::create can build the link
  // (`{base_url}/register?invite_code=<code>`); it errors without a base URL.
  await seedConfigViaTool("site.registration_policy", "operator_invites");
  await seedConfigViaTool("site.base_url", "https://example.com");

  // The operator sends an invite to this test's mailbox recipient via the
  // /invites UI (shows a "Page not found." fallback unless operator_invites, which
  // we just set).
  await signInAs(page, "testoperator");
  await goto(page, "/invites");
  await expect(page.locator('a[href="/invites"]')).toBeVisible();
  await page.fill('input[name="recipient_email"]', user.email);
  await page.fill('input[name="expires_in_hours"]', "37");
  await click(page, SEL.submit);
  await waitForSelector(page, 'p:has-text("Invitation emailed to")');

  // Read the invitation email and pull the code out of the link.
  const email = await mailbox.waitForNewEmail();
  const code = extractInviteCode(email);

  // A fresh, logged-out visitor follows the invite link and registers. No code
  // is typed — the register page carries it from the URL as a hidden field.
  // Holdout (spec D6): invite-gated registration (#433) through the real UI.
  const context = await tracedContext();
  try {
    const invitee = await context.newPage();
    const firstNav = slowBrowserFirstNavigationTimeoutMs(test.info(), 15_000);
    const username = generateUsername("invitee");
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

test("invite creation pending prevents duplicate dispatch", async ({
  page,
  user,
}) => {
  await seedConfigViaTool("site.registration_policy", "operator_invites");
  await seedConfigViaTool("site.base_url", "https://example.com");
  await signInAs(page, "testoperator");
  await goto(page, "/invites");
  await waitForSelector(page, 'input[name="recipient_email"]');

  let createRequests = 0;
  let listRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/invites/create")) createRequests += 1;
    if (request.url().includes("/api/invites/list")) listRequests += 1;
  });
  const release = await stallServerFn(page, "invites/create");
  await page.fill('input[name="recipient_email"]', user.email);
  await page.fill('input[name="expires_in_hours"]', "37");
  await click(page, SEL.submit);
  await expect.poll(() => createRequests).toBe(1);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await page.locator('input[name="expires_in_hours"]').press("Enter");
  expect(createRequests).toBe(1);
  release();

  await waitForSelector(page, 'p:has-text("Invitation emailed to")');
  await expect.poll(() => listRequests).toBe(1);
});

test("invite creation server failure renders error", async ({ page, user }) => {
  await seedConfigViaTool("site.registration_policy", "operator_invites");
  await seedConfigViaTool("site.base_url", "https://example.com");
  await signInAs(page, "testoperator");
  await goto(page, "/invites");
  await waitForSelector(page, 'input[name="recipient_email"]');

  let listRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/invites/list")) listRequests += 1;
  });
  await failServerFn(page, "invites/create");
  await page.fill('input[name="recipient_email"]', user.email);
  await page.fill('input[name="expires_in_hours"]', "37");
  await click(page, SEL.submit);

  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator('p:has-text("Invitation emailed to")')).toHaveCount(
    0,
  );
  expect(listRequests).toBe(0);
});

test("invite creation invalid fields do not dispatch", async ({ page }) => {
  await seedConfigViaTool("site.registration_policy", "operator_invites");
  await seedConfigViaTool("site.base_url", "https://example.com");
  await signInAs(page, "testoperator");
  await goto(page, "/invites");

  let createRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/invites/create")) createRequests += 1;
  });
  const recipient = page.locator('input[name="recipient_email"]');
  const expiry = page.locator('input[name="expires_in_hours"]');
  await recipient.fill("not-an-email");
  await recipient.blur();
  await expiry.fill("337");
  await expiry.blur();

  await expect(page.locator(SEL.error)).toHaveCount(2);
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await expiry.press("Enter");
  expect(createRequests).toBe(0);
});

// Test B — no-code guidance: either invitation policy requires an invitation link before
// registration, so `/register` without one shows guidance rather than a submit button.
for (const policy of ["operator_invites", "member_invites"]) {
  test(`invitation-required /register with no code shows guidance under ${policy}`, async ({
    page,
  }) => {
    // Holdout (spec D6): the invitation-required guidance branch for each issuer role.
    await seedConfigViaTool("site.registration_policy", policy);
    const firstNav = slowBrowserFirstNavigationTimeoutMs(test.info(), 15_000);

    await goto(page, "/register", { timeout: firstNav });

    await expect(
      page.locator('p:has-text("You need an invitation link to register")'),
    ).toBeVisible();
    // The guidance branch replaces the whole form — no register submit button.
    await expect(
      page.locator('.j-page-narrow button[type="submit"]'),
    ).toHaveCount(0);
  });
}

// Test C — policy guards: unavailable policies hide the navigation link and render the
// client-side fallback rather than an SSR 404.
for (const policy of ["closed", "open"]) {
  test(`invites page is unavailable without navigation under ${policy}`, async ({
    page,
  }) => {
    await seedConfigViaTool("site.registration_policy", policy);
    const firstNav = slowBrowserFirstNavigationTimeoutMs(test.info(), 15_000);

    await signInAs(page, "testoperator");
    await goto(page, "/invites", { timeout: firstNav });

    await expect(page.locator('p:has-text("Page not found.")')).toBeVisible();
    await expect(page.locator('input[name="recipient_email"]')).toHaveCount(0);
    await expect(page.locator('a[href="/invites"]')).toHaveCount(0);
  });
}

test("operator-invites denies members and hides navigation", async ({
  page,
}) => {
  await seedConfigViaTool("site.registration_policy", "operator_invites");
  await signInAs(page, "testlogin");

  await goto(page, "/invites");

  await expect(page.locator('p:has-text("Page not found.")')).toBeVisible();
  await expect(page.locator('input[name="recipient_email"]')).toHaveCount(0);
  await expect(page.locator('a[href="/invites"]')).toHaveCount(0);
});

test("member-invites members issue invitations without fetching the ledger", async ({
  page,
}) => {
  await seedConfigViaTool("site.registration_policy", "member_invites");
  await signInAs(page, "testlogin");

  let listRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/invites/list")) listRequests += 1;
  });
  await goto(page, "/invites");

  await waitForSelector(page, 'input[name="recipient_email"]');
  await expect(page.locator('a[href="/invites"]')).toBeVisible();
  expect(listRequests).toBe(0);
  await expect(page.locator(".j-page > ul")).toHaveCount(0);
});

test("member-invites operators issue invitations and see the ledger", async ({
  page,
}) => {
  await seedConfigViaTool("site.registration_policy", "member_invites");
  await signInAs(page, "testoperator");

  let listRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/invites/list")) listRequests += 1;
  });
  await goto(page, "/invites");

  await waitForSelector(page, 'input[name="recipient_email"]');
  await expect(page.locator('a[href="/invites"]')).toBeVisible();
  await expect.poll(() => listRequests).toBe(1);
  await expect(page.locator(".j-page > ul")).toHaveCount(1);
});

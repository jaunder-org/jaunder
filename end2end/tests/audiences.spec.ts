import { test, expect } from "./fixtures";
import {
  goto,
  click,
  signInAsNewUser,
  subscribeTo,
  failServerFn,
  stallServerFn,
} from "./helpers";

// Audiences management UI (`/audiences`, converged into `web::audiences`).
//
// Guards the reactive re-fetch behaviour of the decomposed screen — hazards the
// `#[component]` coverage exemption leaves to e2e:
//   1. A membership toggle must NOT rebuild (remount) the audience list, and must
//      re-fetch only that audience's members (#359). Each MemberChecklist owns a *local*
//      members trigger, so an add/remove re-fetches only its own audience. Verified with a
//      stable element handle on an untouched row + a `audiences::list_members` request count.
//   2. A list-level mutation (create/rename/delete) must `patch` the keyed reactive store
//      in place (#348): unchanged rows keep their DOM — their MemberChecklists are never
//      remounted (no "Loading members…" reflash) — and a rename updates the row's name in
//      place. Verified with stable element handles on the rows' checklist <ul>s held across
//      create, rename, and delete.
//   3. A mutation must not blank content into a "Loading…" flash: resolved values are
//      retained across a re-fetch (sticky signals / store patch-on-success).
// The happy-path CRUD is exercised through the real forms along the way.

test("Audiences: CRUD + membership toggle re-fetch without list remount or flash", async ({
  page,
  tracedContext,
}) => {
  const author = await signInAsNewUser(page);

  // A subscriber X so the author has someone to add to an audience.
  const xCtx = await tracedContext();
  const xPage = await xCtx.newPage();
  const userX = await signInAsNewUser(xPage);
  await subscribeTo(xPage, author);
  await xCtx.close();

  await goto(page, "/audiences");

  // Create two audiences. The second create re-fetches the list; the first must
  // survive (sticky list, no flash-to-empty).
  const createName = 'input[placeholder="Audience name"]';
  await page.fill(createName, "Friends");
  await click(page, 'button:has-text("Create")');
  await expect(
    page.locator(".j-audience-item", { hasText: "Friends" }),
  ).toBeVisible();

  await page.fill(createName, "Family");
  await click(page, 'button:has-text("Create")');
  const friends = page.locator(".j-audience-item", { hasText: "Friends" });
  const family = page.locator(".j-audience-item", { hasText: "Family" });
  await expect(friends).toBeVisible();
  await expect(family).toBeVisible();

  // Stable handle on the *Family* name node. Adding a member to *Friends* must not
  // remount Family — single-signal coupling would rebuild the whole list and
  // detach this node.
  const familyName = await family.locator("h3.j-audience-name").elementHandle();

  // X is an addable candidate in BOTH audiences (a subscriber, member of neither).
  // Wait for both checklists so the initial member fetches finish before counting.
  const friendsX = friends
    .locator(".j-audience-members li")
    .filter({ hasText: userX });
  const familyX = family
    .locator(".j-audience-members li")
    .filter({ hasText: userX });
  await expect(friendsX.locator('button:has-text("Add")')).toBeVisible();
  await expect(familyX.locator('button:has-text("Add")')).toBeVisible();

  // #348: stable handles on each row's checklist <ul>. A list-level refetch must `patch`
  // the keyed store in place, so these exact DOM nodes survive create/rename/delete of
  // *other* rows (and of this row, on rename) — a rebuild would detach them.
  const friendsChecklist = await friends
    .locator("ul.j-audience-members")
    .elementHandle();
  const familyChecklist = await family
    .locator("ul.j-audience-members")
    .elementHandle();

  // The members trigger is local to each MemberChecklist, so adding X to Friends
  // re-fetches ONLY Friends' members — one `audiences::list_members` round-trip. A
  // shared trigger would produce two (Friends + Family).
  // Two request counts. `memberFetches`: a local per-checklist trigger, so a toggle
  // re-fetches only its own audience (one round-trip). `listFetches`: the audience LIST
  // must NOT re-fetch on a membership toggle (the scoped-invalidation guard) — its scope
  // fires only on create/rename/delete, and only on *success*.
  let memberFetches = 0;
  let listFetches = 0;
  page.on("request", (req) => {
    const url = req.url();
    if (url.includes("/api/audiences/list_members")) memberFetches += 1;
    if (url.includes("/api/audiences/list_mine")) listFetches += 1;
  });

  // Add X to Friends; the button flips Add -> Remove.
  await friendsX.locator('button:has-text("Add")').click();
  await expect(friendsX.locator('button:has-text("Remove")')).toBeVisible();

  // The untouched Family row was NOT remounted by the member toggle.
  expect(await familyName!.evaluate((el) => el.isConnected)).toBe(true);
  // Only Friends' checklist re-fetched (local trigger), not Family's.
  expect(memberFetches).toBe(1);
  // The audience LIST did NOT re-fetch on the membership toggle — scoped invalidation.
  // A single shared invalidator (over-invalidating) would have re-fetched it here.
  expect(listFetches).toBe(0);
  // No members list left stuck on the loading placeholder.
  await expect(page.getByText("Loading members")).toHaveCount(0);

  // Remove X; the button flips back.
  await friendsX.locator('button:has-text("Remove")').click();
  await expect(friendsX.locator('button:has-text("Add")')).toBeVisible();
  // The remove re-fetches only Friends too: one additional request despite two
  // mounted checklists, and still no audience-list refresh.
  expect(memberFetches).toBe(2);
  expect(listFetches).toBe(0);

  // #348 (create): creating another audience refetches the list; the keyed store `patch`es
  // in place, so the two existing rows' checklists are not remounted (handles stay
  // connected). The new "Extras" row loads its own checklist, so a global "Loading members"
  // count would be a false negative here — the per-row handles are the real observable.
  await page.fill(createName, "Extras");
  await click(page, 'button:has-text("Create")');
  await expect(
    page.locator(".j-audience-item", { hasText: "Extras" }),
  ).toBeVisible();
  expect(await friendsChecklist!.evaluate((el) => el.isConnected)).toBe(true);
  expect(await familyChecklist!.evaluate((el) => el.isConnected)).toBe(true);

  // Rename Friends -> BestFriends; the list re-fetches (a `list` bump) and both
  // audiences remain.
  const renameForm = friends.locator("form").filter({ hasText: "Rename" });
  await renameForm.locator('input[name="name"]').fill("BestFriends");
  await renameForm.locator('button:has-text("Rename")').click();
  await expect(
    page.locator("h3.j-audience-name", { hasText: "BestFriends" }),
  ).toBeVisible();
  await expect(family).toBeVisible();
  // The rename re-fetched the list (its own scope fired), so the guard above is a live
  // counter — it stayed at 0 on the toggle because of scoping, not because it never moves.
  expect(listFetches).toBeGreaterThanOrEqual(1);
  // #348 (rename in place): the renamed row updated its <h3> to the new name WITHOUT being
  // remounted — its checklist <ul> is the same DOM node (handle still connected), as is the
  // unrelated Family one. Keying on audience_id + a reactive name subfield is what updates
  // the name in place instead of rebuilding the row (which would reflash its members).
  expect(await friendsChecklist!.evaluate((el) => el.isConnected)).toBe(true);
  expect(await familyChecklist!.evaluate((el) => el.isConnected)).toBe(true);

  // #348 (delete): deleting one audience removes only its row; the others' checklists are
  // not remounted. Delete "Extras"; Family's checklist node survives.
  const extras = page.locator(".j-audience-item", { hasText: "Extras" });
  await extras.locator('button:has-text("Delete")').click();
  await expect(
    page.locator(".j-audience-item", { hasText: "Extras" }),
  ).toHaveCount(0);
  expect(await familyChecklist!.evaluate((el) => el.isConnected)).toBe(true);

  // Success-gating: a FAILED create (duplicate name) must NOT fire the list invalidator,
  // so the list does not re-fetch. Record the count, attempt the dup, assert it's flat.
  const beforeDup = listFetches;
  await page.fill(createName, "BestFriends");
  await click(page, 'button:has-text("Create")');
  // Any create error will do — the point is that a failed create does not refetch. Not
  // coupled to the exact store message (rewording it shouldn't hang this to a timeout).
  await expect(page.locator("p.error")).toBeVisible();
  expect(listFetches).toBe(beforeDup);
});

test("audience rename pending and error preserve the row", async ({ page }) => {
  await signInAsNewUser(page);
  await goto(page, "/audiences");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  const row = page.locator(".j-audience-item", { hasText: "Friends" });
  await expect(row).toBeVisible();
  const form = row.locator("form").filter({ hasText: "Rename" });
  const input = form.locator('input[name="name"]');
  const button = form.locator('button:has-text("Rename")');

  let renameRequests = 0;
  let listRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/audiences/rename")) renameRequests += 1;
    if (request.url().includes("/api/audiences/list_mine")) listRequests += 1;
  });
  await input.fill("   ");
  await input.blur();
  await expect(
    form.locator("p.error", {
      hasText: "audience name must not be empty",
    }),
  ).toBeVisible();
  await expect(button).toBeDisabled();
  await input.press("Enter");
  expect(renameRequests).toBe(0);

  const release = await stallServerFn(page, "audiences/rename");
  await input.fill("BestFriends");
  await button.click();
  await expect.poll(() => renameRequests).toBe(1);
  await expect(button).toBeDisabled();
  await input.press("Enter");
  expect(renameRequests).toBe(1);
  release();
  await expect(
    page.locator("h3.j-audience-name", { hasText: "BestFriends" }),
  ).toBeVisible();
  await expect.poll(() => listRequests).toBe(1);

  await failServerFn(page, "audiences/rename");
  await input.fill("StillFriends");
  await button.click();
  await expect(form.locator("p.error")).toBeVisible();
  await expect(
    page.locator("h3.j-audience-name", { hasText: "BestFriends" }),
  ).toBeVisible();
  expect(listRequests).toBe(1);
});

test("audience add pending prevents duplicate dispatch", async ({
  page,
  tracedContext,
}) => {
  const author = await signInAsNewUser(page);
  const subscriberContext = await tracedContext();
  const subscriberPage = await subscriberContext.newPage();
  const subscriber = await signInAsNewUser(subscriberPage);
  await subscribeTo(subscriberPage, author);
  await subscriberContext.close();
  await goto(page, "/audiences");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  const row = page
    .locator(".j-audience-item", { hasText: "Friends" })
    .locator(".j-audience-members li")
    .filter({ hasText: subscriber });
  const button = row.locator('button:has-text("Add")');
  await expect(button).toBeVisible();

  let addRequests = 0;
  let memberRequests = 0;
  let listRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/audiences/add_subscriber"))
      addRequests += 1;
    if (request.url().includes("/api/audiences/list_members"))
      memberRequests += 1;
    if (request.url().includes("/api/audiences/list_mine")) listRequests += 1;
  });
  await failServerFn(page, "audiences/add_subscriber");
  await button.click();
  await expect(row.locator("p.error")).toBeVisible();
  expect(memberRequests).toBe(0);
  expect(listRequests).toBe(0);
  await page.unroute("**/api/audiences/add_subscriber");
  addRequests = 0;
  const release = await stallServerFn(page, "audiences/add_subscriber");
  await button.click();
  await expect.poll(() => addRequests).toBe(1);
  await expect(button).toBeDisabled();
  await button.press("Enter");
  expect(addRequests).toBe(1);
  release();
  await expect(row.locator('button:has-text("Remove")')).toBeVisible();
  await expect.poll(() => memberRequests).toBe(1);
  expect(listRequests).toBe(0);
});

test("audience remove pending prevents duplicate dispatch", async ({
  page,
  tracedContext,
}) => {
  const author = await signInAsNewUser(page);
  const subscriberContext = await tracedContext();
  const subscriberPage = await subscriberContext.newPage();
  const subscriber = await signInAsNewUser(subscriberPage);
  await subscribeTo(subscriberPage, author);
  await subscriberContext.close();
  await goto(page, "/audiences");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  await page.fill('input[placeholder="Audience name"]', "Family");
  await click(page, 'button:has-text("Create")');
  const friends = page.locator(".j-audience-item", { hasText: "Friends" });
  const family = page.locator(".j-audience-item", { hasText: "Family" });
  const familySubscriber = family
    .locator(".j-audience-members li")
    .filter({ hasText: subscriber });
  await expect(
    familySubscriber.locator('button:has-text("Add")'),
  ).toBeVisible();
  const friendsId = await friends
    .locator('input[name="audience_id"]')
    .inputValue();
  const familyId = await family
    .locator('input[name="audience_id"]')
    .inputValue();
  const row = friends
    .locator(".j-audience-members li")
    .filter({ hasText: subscriber });
  await row.locator('button:has-text("Add")').click();
  const button = row.locator('button:has-text("Remove")');
  await expect(button).toBeVisible();

  let removeRequests = 0;
  let targetMemberRequests = 0;
  let unrelatedMemberRequests = 0;
  let listRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/audiences/remove_subscriber"))
      removeRequests += 1;
    if (request.url().includes("/api/audiences/list_members")) {
      const audienceId = new URLSearchParams(request.postData() ?? "").get(
        "audience_id",
      );
      if (audienceId === friendsId) targetMemberRequests += 1;
      if (audienceId === familyId) unrelatedMemberRequests += 1;
    }
    if (request.url().includes("/api/audiences/list_mine")) listRequests += 1;
  });
  await failServerFn(page, "audiences/remove_subscriber");
  await button.click();
  await expect(row.locator("p.error")).toBeVisible();
  expect(targetMemberRequests).toBe(0);
  expect(unrelatedMemberRequests).toBe(0);
  expect(listRequests).toBe(0);
  await page.unroute("**/api/audiences/remove_subscriber");
  removeRequests = 0;
  const release = await stallServerFn(page, "audiences/remove_subscriber");
  await button.click();
  await expect.poll(() => removeRequests).toBe(1);
  await expect(button).toBeDisabled();
  await button.press("Enter");
  expect(removeRequests).toBe(1);
  release();
  await expect(row.locator('button:has-text("Add")')).toBeVisible();
  await expect.poll(() => targetMemberRequests).toBe(1);
  expect(unrelatedMemberRequests).toBe(0);
  expect(listRequests).toBe(0);
});
// #383: fetch-error UI branches, driven by Playwright route interception (`failServerFn`).
// A read server-fn only `Err`s if the DB breaks, so these error nodes — which the
// `#[component]`/`cov:ignore` exemptions push to e2e — were otherwise unexercised. The
// server fn never runs; the client `Resource` resolves `Err` and the error branch renders.

test("Audiences: a list fetch error surfaces the error node, not an empty list", async ({
  page,
}) => {
  await signInAsNewUser(page);

  // Force the audience-list resource to fail before the page loads it.
  await failServerFn(page, "audiences/list_mine");
  await goto(page, "/audiences");

  // `ListState::Error` renders `<p class="error">` — NOT the empty-state "No audiences yet."
  // (which would mean the error was swallowed to an empty list).
  await expect(page.locator("p.error")).toBeVisible();
  await expect(page.getByText("No audiences yet.")).toHaveCount(0);
});

test("Audiences: a members fetch error surfaces the error node, not an empty checklist", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/audiences");

  // Force the members resource to fail, then create an audience whose checklist will fetch.
  await failServerFn(page, "audiences/list_members");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');

  // `MemberChecklist`'s `sticky` resolves `Some(Err)` → its error node renders (the branch
  // #372 added), NOT an empty checklist / "No active subscribers yet." (which would mean the
  // error was swallowed to an empty member set — the #346 defect class this guards against).
  const friends = page.locator(".j-audience-item", { hasText: "Friends" });
  await expect(friends.locator("p.error")).toBeVisible();
  await expect(friends.getByText("No active subscribers yet.")).toHaveCount(0);
});

// #346: a failed `list_my_subscribers` fetch must surface an error, not masquerade as an
// empty roster (which rendered every subscriber's row as "nobody is a member"). Uses the
// shared `failServerFn` fault-injection helper (#383, which left this roster branch to #346).
test("Audiences: a failed subscriber-roster fetch surfaces an error, not an empty roster", async ({
  page,
  tracedContext,
}) => {
  const author = await signInAsNewUser(page);

  // A real subscriber X, so an empty roster would be a lie — the exact #346 bug.
  const xCtx = await tracedContext();
  const xPage = await xCtx.newPage();
  await signInAsNewUser(xPage);
  await subscribeTo(xPage, author);
  await xCtx.close();

  // Force the roster fetch to fail before the page loads it (the shared #383 helper).
  await failServerFn(page, "audiences/list_my_subscribers");

  await goto(page, "/audiences");

  // AC1: the roster error surfaces once, at page level — visible even before any audience
  // exists (zero audiences ⇒ no rows ⇒ no per-row checklist to carry the error).
  await expect(page.getByText("Couldn't load your subscribers")).toBeVisible();

  // Create an audience so a MemberChecklist mounts against the failed roster.
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  await expect(
    page.locator(".j-audience-item", { hasText: "Friends" }),
  ).toBeVisible();
  // Let the checklist settle past its own members-loading state before asserting.
  await expect(page.getByText("Loading members")).toHaveCount(0);

  // AC2: no empty-roster lie — no "No active subscribers yet." and no add/remove
  // list, despite X being a real subscriber.
  await expect(page.getByText("No active subscribers yet")).toHaveCount(0);
  await expect(page.locator(".j-audience-members")).toHaveCount(0);
});

// #346 AC3: a genuinely empty roster (author with no subscribers) must stay distinct from
// the error state — it still shows the empty message and no error node.
test("Audiences: a genuinely empty roster still shows the empty message", async ({
  page,
}) => {
  await signInAsNewUser(page);

  await goto(page, "/audiences");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  await expect(
    page.locator(".j-audience-item", { hasText: "Friends" }),
  ).toBeVisible();
  await expect(page.getByText("Loading members")).toHaveCount(0);

  await expect(page.getByText("No active subscribers yet")).toBeVisible();
  await expect(page.getByText("Couldn't load your subscribers")).toHaveCount(0);
});

// #347: the subscriber roster is fetched once at page load; a mid-session new subscriber
// must be pullable via the in-page refresh control (no full reload). Real subscribe event
// in a second context — no fault injection.
test("Audiences: refresh pulls a mid-session new subscriber into the checklists", async ({
  page,
  tracedContext,
}) => {
  const author = await signInAsNewUser(page);

  await goto(page, "/audiences");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  const friends = page.locator(".j-audience-item", { hasText: "Friends" });
  await expect(friends).toBeVisible();
  // Roster fetched empty at load: once the checklist settles it shows the empty message.
  await expect(page.getByText("Loading members")).toHaveCount(0);
  await expect(friends.getByText("No active subscribers yet.")).toBeVisible();

  // A subscriber arrives mid-session (another user's session).
  const xCtx = await tracedContext();
  const xPage = await xCtx.newPage();
  const userX = await signInAsNewUser(xPage);
  await subscribeTo(xPage, author);
  await xCtx.close();

  // The once-fetched roster is stale — X hasn't appeared, so the checklist still shows the
  // empty message. (Asserting the absent candidate `<ul>` would pass vacuously: an empty
  // roster renders `<p>`, not a `<ul class="j-audience-members">`.)
  await expect(friends.getByText("No active subscribers yet.")).toBeVisible();

  // Click the refresh control (by accessible name); X appears as an "Add" candidate — no reload.
  await page.getByRole("button", { name: "Refresh subscribers" }).click();
  const friendsX = friends
    .locator(".j-audience-members li")
    .filter({ hasText: userX });
  await expect(friendsX.locator('button:has-text("Add")')).toBeVisible();
});

// #350: the audience name is a typed `AudienceName` wire arg with client-side
// pre-validation (ADR-0065, direct-bind). The create form must gate submit
// disable-until-valid and show the newtype's own message inline once touched — a valid
// name never reaches the (malicious-only) decode-time rejection.
test("Audiences: create-name client-side validation gates submit", async ({
  page,
}) => {
  await signInAsNewUser(page);

  await goto(page, "/audiences");
  const nameInput = 'input[placeholder="Audience name"]';
  const createBtn = 'button:has-text("Create")';

  // Pristine empty name: the non-empty rule leaves Create disabled (no `required` attr).
  await expect(page.locator(createBtn)).toBeDisabled();

  // A whitespace-only name is invalid; blurring (touch) surfaces the newtype's message.
  await page.fill(nameInput, "   ");
  await page.locator(nameInput).blur();
  await expect(
    page.locator("p.error", { hasText: "audience name must not be empty" }),
  ).toBeVisible();
  await expect(page.locator(createBtn)).toBeDisabled();

  // A valid name clears the error, enables submit, and creates the audience.
  await page.fill(nameInput, "Friends");
  await expect(page.locator(createBtn)).toBeEnabled();
  await click(page, createBtn);
  await expect(
    page.locator(".j-audience-item", { hasText: "Friends" }),
  ).toBeVisible();
});

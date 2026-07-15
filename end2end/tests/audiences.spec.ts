import {
  test,
  expect,
  setTestBudget,
  slowBrowserFirstNavigationTimeoutMs,
} from "./fixtures";
import { goto, click, register, subscribeTo, failServerFn } from "./helpers";

// Audiences management UI (`/audiences`, converged into `web::audiences`).
//
// Guards the reactive re-fetch behaviour of the decomposed screen — hazards the
// `#[component]` coverage exemption leaves to e2e:
//   1. A membership toggle must NOT rebuild (remount) the audience list, and must
//      re-fetch only that audience's members (#359). Each MemberChecklist owns a *local*
//      members trigger, so an add/remove re-fetches only its own audience. Verified with a
//      stable element handle on an untouched row + a `list_audience_members` request count.
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
  browser,
}, testInfo) => {
  setTestBudget(120_000);
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);

  const author = await register(page, firstNav);

  // A subscriber X so the author has someone to add to an audience.
  const xCtx = await browser.newContext();
  const xPage = await xCtx.newPage();
  const userX = await register(xPage, firstNav);
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
  // remount Family; with the old single-signal coupling the whole list rebuilt and
  // this node would detach.
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
  // re-fetches ONLY Friends' members — one `list_audience_members` round-trip. A
  // shared trigger would produce two (Friends + Family).
  // Two request counts. `memberFetches`: a local per-checklist trigger, so a toggle
  // re-fetches only its own audience (one round-trip). `listFetches`: the audience LIST
  // must NOT re-fetch on a membership toggle (the scoped-invalidation guard) — its scope
  // fires only on create/rename/delete, and only on *success*.
  let memberFetches = 0;
  let listFetches = 0;
  page.on("request", (req) => {
    const url = req.url();
    if (url.includes("/api/list_audience_members")) memberFetches += 1;
    if (url.includes("/api/list_my_audiences")) listFetches += 1;
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

// #383: fetch-error UI branches, driven by Playwright route interception (`failServerFn`).
// A read server-fn only `Err`s if the DB breaks, so these error nodes — which the
// `#[component]`/`cov:ignore` exemptions push to e2e — were otherwise unexercised. The
// server fn never runs; the client `Resource` resolves `Err` and the error branch renders.

test("Audiences: a list fetch error surfaces the error node, not an empty list", async ({
  page,
}, testInfo) => {
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  await register(page, firstNav);

  // Force the audience-list resource to fail before the page loads it.
  await failServerFn(page, "list_my_audiences");
  await goto(page, "/audiences");

  // `ListState::Error` renders `<p class="error">` — NOT the empty-state "No audiences yet."
  // (which would mean the error was swallowed to an empty list).
  await expect(page.locator("p.error")).toBeVisible();
  await expect(page.getByText("No audiences yet.")).toHaveCount(0);
});

test("Audiences: a members fetch error surfaces the error node, not an empty checklist", async ({
  page,
}, testInfo) => {
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  await register(page, firstNav);
  await goto(page, "/audiences");

  // Force the members resource to fail, then create an audience whose checklist will fetch.
  await failServerFn(page, "list_audience_members");
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
  browser,
}, testInfo) => {
  setTestBudget(120_000);
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  const author = await register(page, firstNav);

  // A real subscriber X, so an empty roster would be a lie — the exact #346 bug.
  const xCtx = await browser.newContext();
  const xPage = await xCtx.newPage();
  await register(xPage, firstNav);
  await subscribeTo(xPage, author);
  await xCtx.close();

  // Force the roster fetch to fail before the page loads it (the shared #383 helper).
  await failServerFn(page, "list_my_subscribers");

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

  // AC2: the empty-roster lie is gone — no "No active subscribers yet." and no add/remove
  // list, despite X being a real subscriber.
  await expect(page.getByText("No active subscribers yet")).toHaveCount(0);
  await expect(page.locator(".j-audience-members")).toHaveCount(0);
});

// #346 AC3: a genuinely empty roster (author with no subscribers) must stay distinct from
// the error state — it still shows the empty message and no error node.
test("Audiences: a genuinely empty roster still shows the empty message", async ({
  page,
}, testInfo) => {
  setTestBudget(120_000);
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  await register(page, firstNav);

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
  browser,
}, testInfo) => {
  setTestBudget(120_000);
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  const author = await register(page, firstNav);

  await goto(page, "/audiences");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  const friends = page.locator(".j-audience-item", { hasText: "Friends" });
  await expect(friends).toBeVisible();
  // Roster fetched empty at load: once the checklist settles it shows the empty message.
  await expect(page.getByText("Loading members")).toHaveCount(0);
  await expect(friends.getByText("No active subscribers yet.")).toBeVisible();

  // A subscriber arrives mid-session (another user's session).
  const xCtx = await browser.newContext();
  const xPage = await xCtx.newPage();
  const userX = await register(xPage, firstNav);
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
}, testInfo) => {
  setTestBudget(60_000);
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  await register(page, firstNav);

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

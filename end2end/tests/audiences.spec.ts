import {
  test,
  expect,
  setTestBudget,
  slowBrowserFirstNavigationTimeoutMs,
} from "./fixtures";
import { goto, click, register, subscribeTo } from "./helpers";

// Audiences management UI (`/audiences`, converged into `web::audiences`).
//
// Guards the reactive re-fetch behaviour of the decomposed screen — two hazards
// the decomposition introduced and that the `#[component]` coverage exemption
// leaves to e2e:
//   1. A membership toggle must NOT rebuild (remount) the audience list, and must
//      re-fetch only that audience's members. The list re-fetch is a context signal
//      (create/rename/delete); each MemberChecklist owns a *local* members trigger,
//      so an add/remove re-fetches only its own audience. Verified with a stable
//      element handle on an untouched row + a `list_audience_members` request count.
//   2. A mutation must not blank content into a "Loading…" flash: resolved values
//      are held in sticky signals across a re-fetch.
// The happy-path CRUD is exercised through the real forms along the way.

test("Audiences: CRUD + membership toggle re-fetch without list remount or flash", async ({
  page,
  browser,
}, testInfo) => {
  setTestBudget(90_000);
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

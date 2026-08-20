# Audience Refetch Stability Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Audience E2E tests fail if a successful refetch transiently
unmounts prior data or renders a loading placeholder.

**Architecture:** Add selective, per-test Playwright route gates around the read
requests dispatched after successful mutations. The gates retain real backend
responses, release only after assertions on the in-flight DOM state, and keep
interception local because no existing shared helper supports safe request-body
selection. No application code changes.

**Tech Stack:** TypeScript, Playwright, existing `end2end/tests` fixtures and
helpers, `cargo xtask e2e-local`.

## Global Constraints

- Follow the approved
  [spec](../specs/2026-08-20-issue-357-audience-no-flash.md), especially
  Decisions D1–D4 and AC1–AC6.
- Use `goto`, `click`, and `waitForSelector` from `./helpers`; do not add a
  document load, `networkidle` wait, sleep, retry, or a navigation exemption.
- Register each route only after initial resource loads settle and immediately
  before the mutation it observes. Wait for its target request to arrive before
  inspecting the in-flight DOM, then call its release function before awaiting
  the final state.
- A `list_members` route must parse `route.request().postData()` with
  `URLSearchParams` and stall only its named `audience_id`; every other request
  must `route.continue()` immediately.
- Preserve the existing request-count, error, pending-state, and final-state
  assertions. Production code, endpoints, and shared helpers remain unchanged.
- Before each commit, tick its task checkbox, run
  `devtool run -- cargo xtask check`, stage any mechanical formatting changes,
  then commit with no `Co-Authored-By` trailer.

---

## Review Header

**Scope in:** in-flight membership and audience-list stability assertions in
`audiences.spec.ts`; in-flight membership stability in the existing
named-Audience publishing scenario.

**Scope out:** production reactive changes; a shared selective-stall helper; new
navigation, visual, accessibility, backend, or endpoint coverage.

**Tasks:**

1. Add deterministic read-refetch gates and transient DOM assertions to the
   Audience CRUD/membership test.
2. Add the same membership-transition proof to the named-Audience publish flow.

**Key risks/decisions:** The route must not catch initial or unrelated reads;
request-arrival counters prevent a false assertion before the resource starts
loading. New rows can legitimately have their own initial loading state, so
list-level checks observe pre-existing row handles and list-level branches, not
a global `Loading members…` count.

## File Structure

| File                               | Responsibility                                                                                                              |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `end2end/tests/audiences.spec.ts`  | Selectively stall post-mutation `list_members` and `list_mine` reads and assert the existing Audience DOM during each hold. |
| `end2end/tests/visibility.spec.ts` | Prove the Add-member refetch remains stable in the named-Audience publishing flow.                                          |

### Task 1: Prove Audience page refetch stability

**Files:**

- Modify: `end2end/tests/audiences.spec.ts:28-171`
- Test: `end2end/tests/audiences.spec.ts:28-171`

**Interfaces:**

- Consumes: `click(page, selector)`, `goto(page, path)`, and `expect` from the
  existing test; `Route`'s `request().postData()` and `continue()` APIs.
- Produces: local `listMembersGate` / `listMineGate` route handlers in the
  existing CRUD-and-membership test. Each exposes a target-request count and a
  release closure; it forwards non-target requests immediately.

- [x] **Step 1: Add failing in-flight membership assertions**

  After both initial Add buttons are visible, read `friendsId` and `familyId`
  from each row's `input[name="audience_id"]`. Before clicking Friends' Add
  button, register:

  ```ts
  let targetMemberFetches = 0;
  let releaseMembers!: () => void;
  const membersGate = new Promise<void>((resolve) => {
    releaseMembers = resolve;
  });
  await page.route("**/api/audiences/list_members", async (route) => {
    const audienceId = new URLSearchParams(
      route.request().postData() ?? "",
    ).get("audience_id");
    if (audienceId !== friendsId) return route.continue();
    targetMemberFetches += 1;
    await membersGate;
    await route.continue();
  });
  ```

  Before clicking Add, capture the pre-refetch target row:

  ```ts
  const friendsXHandle = await friendsX.elementHandle();
  expect(friendsXHandle).not.toBeNull();
  ```

  After `await expect.poll(() => targetMemberFetches).toBe(1)`, assert while the
  route is still held:

  ```ts
  expect(await friendsChecklist!.evaluate((el) => el.isConnected)).toBe(true);
  expect(await familyChecklist!.evaluate((el) => el.isConnected)).toBe(true);
  expect(await friendsXHandle!.evaluate((el) => el.isConnected)).toBe(true);
  await expect(friendsX.locator('button:has-text("Add")')).toBeVisible();
  await expect(familyX.locator('button:has-text("Add")')).toBeVisible();
  await expect(page.getByText("Loading members")).toHaveCount(0);
  ```

  Release and retain the existing Remove final-state and request-scope checks.
  Before Remove, capture its now-member `friendsX` row handle; after the held
  target request arrives, assert that handle and both checklist handles remain
  connected and that its Remove button remains visible. Release and assert Add.
  Record and assert that no Family `list_members` request is gated or counted.
  These regression assertions fail if `sticky` clears the target resource or the
  membership invalidator is widened.

- [x] **Step 2: Run the focused test**

  Run: `devtool run -- cargo xtask e2e-local audiences.spec.ts`

  Expected: PASS. The held Add and Remove refetches reach their routes, prior
  rows stay connected and non-loading during each hold, and final button states
  still flip after release.

- [x] **Step 3: Add failing in-flight Audience-list assertions**

  Capture the existing Friends and Family row handles before each list mutation.
  Immediately before the second create, rename, and delete actions, register a
  local route on `**/api/audiences/list_mine` with a fresh counter and release
  gate:

  ```ts
  let listMineFetches = 0;
  let releaseListMine!: () => void;
  const listMineGate = new Promise<void>((resolve) => {
    releaseListMine = resolve;
  });
  await page.route("**/api/audiences/list_mine", async (route) => {
    listMineFetches += 1;
    await listMineGate;
    await route.continue();
  });
  ```

  After the expected request arrives, and before releasing it, assert the
  captured pre-existing row handles are `isConnected`, the rows remain visible,
  `page.getByText("No audiences yet.")` has count zero, and
  `page.locator(".j-audience-list + p.j-loading")` has count zero. Release the
  route, then retain the existing create (Extras visible), rename (BestFriends
  visible), and delete (Extras absent) end-state assertions. Use a fresh
  route/counter per mutation or unroute the released route before registering
  the next one; a previous released route must not make later assertions
  ambiguous.

- [x] **Step 4: Run the focused test**

  Run: `devtool run -- cargo xtask e2e-local audiences.spec.ts`

  Expected: PASS. Each successful list mutation dispatches exactly its held
  read; existing cards persist throughout the hold and reach their intended
  final state after release.

- [x] **Step 5: Commit Audience stability coverage**

  Tick this task complete, run `devtool run -- cargo xtask check`, stage
  `end2end/tests/audiences.spec.ts` plus any formatter changes, and commit:

  ```bash
  git add end2end/tests/audiences.spec.ts
  git commit -m "test(e2e): observe audience refetch stability"
  ```

### Task 2: Prove named-Audience membership stability

**Files:**

- Modify: `end2end/tests/visibility.spec.ts:224-300`
- Test: `end2end/tests/visibility.spec.ts:224-300`

**Interfaces:**

- Consumes: the existing `friends`, `xRow`, `click`, and `waitForSelector`
  values in the named-Audience scenario; the same local selective-route shape
  from Task 1.
- Produces: a held, Friends-only `list_members` read whose release restores the
  existing Remove-button readiness boundary before composition begins.

- [ ] **Step 1: Add the failing in-flight named-Audience assertion**

  Once Friends and X's initial Add button are visible, read Friends'
  `input[name="audience_id"]` and capture its pre-mutation roster:

  ```ts
  const membersList = await friends
    .locator("ul.j-audience-members")
    .elementHandle();
  expect(membersList).not.toBeNull();
  ```

  Before clicking Add, route `**/api/audiences/list_members`; parse each request
  body and continue unless it targets Friends. Count the target request and hold
  it behind a local release promise. Once the target refetch has arrived,
  assert:

  ```ts
  expect(await membersList!.evaluate((el) => el.isConnected)).toBe(true);
  await expect(xRow.locator('button:has-text("Add")')).toBeVisible();
  await expect(page.getByText("Loading members")).toHaveCount(0);
  ```

  Release, then preserve the existing `waitForSelector` for X's Remove button
  and all subsequent post visibility assertions. This must use the existing
  page; do not add a boot.

- [ ] **Step 2: Run the focused test**

  Run: `devtool run -- cargo xtask e2e-local visibility.spec.ts`

  Expected: PASS. The targeted roster stays mounted and non-loading while its
  real post-add refetch is held, then the pre-existing post visibility matrix
  still passes after release.

- [ ] **Step 3: Commit named-Audience stability coverage**

  Tick this task complete, run `devtool run -- cargo xtask check`, stage
  `end2end/tests/visibility.spec.ts` plus any formatter changes, and commit:

  ```bash
  git add end2end/tests/visibility.spec.ts
  git commit -m "test(e2e): hold named audience membership refetch"
  ```

## Verification

- [ ] Run `devtool run -- cargo xtask e2e-local audiences.spec.ts` after Task 1.
- [ ] Run `devtool run -- cargo xtask e2e-local visibility.spec.ts` after
      Task 2.
- [ ] Run `devtool run -- cargo xtask validate` after both commits, before
      shipping. Expected: PASS across all required checks and E2E combinations.

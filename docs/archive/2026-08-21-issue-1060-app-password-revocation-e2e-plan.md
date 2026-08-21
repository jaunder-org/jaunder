# App Password Revocation E2E — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add browser-flow coverage proving an App Password revoked from
`/sessions` disappears from the UI and no longer authenticates AtomPub requests.

**Architecture:** Extend the existing AtomPub e2e spec because it already owns
App Password minting and cookie-free HTTP Basic AtomPub requests. The new test
drives the Sessions UI for create/revoke and uses Playwright's isolated
`request` fixture for the post-revoke Basic-auth probe.

**Tech Stack:** Playwright, TypeScript, Jaunder e2e helpers, AtomPub HTTP Basic.

**Spec:**
[`2026-08-21-issue-1060-app-password-revocation-e2e.md`](../specs/2026-08-21-issue-1060-app-password-revocation-e2e.md)

## Review Header

**Scope — in:** one focused Playwright test in `end2end/tests/atompub.spec.ts`
and the CSR coverage matrix text for App Password management.

**Scope — out:** new product behavior, storage/schema changes, a separate App
Password credential type, direct `/api/sessions/revoke` testing, and changes to
browser-session revocation coverage in `end2end/tests/sessions.spec.ts`.

**Tasks:**

1. Add App Password revocation e2e coverage and update the coverage matrix.

**Key risks/decisions:**

- The credential-death probe must use Playwright's `request` fixture, not
  `page.request`, so it carries no browser `session=` cookie.
- The test must use the mounted `/sessions` UI for revocation; a direct server
  function request would not satisfy the CSR browser-flow gap.
- The credential-death probe targets `GET /atompub/service`, whose server tests
  define unauthenticated AtomPub requests as HTTP 401 with no useful response
  body. Do not borrow the server-fn `"unauthorized"` body assertion here.

## Global Constraints

- Use **App Password** as defined in `CONTEXT.md`: a named,
  individually-revocable credential minted for a Protocol Client.
- Preserve ADR-0014's model: App Passwords are labelled sessions authenticated
  over HTTP Basic for AtomPub.
- Preserve ADR-0111's one-boot-per-page e2e rule: the default page enters once
  at `/sessions`; no second `goto` on that page.
- Use e2e helpers (`goto`, `click`) instead of raw `page.goto` / `page.click`.
- Do not use `page.waitForLoadState("networkidle")`; wait for concrete UI state
  or response state.
- No `Co-Authored-By` trailer.

---

### Task 1: App Password revocation e2e coverage — DONE

**Files:**

- Modify: `end2end/tests/atompub.spec.ts`
- Modify: `docs/coverage/csr-e2e-matrix.md`
- Modify:
  `docs/superpowers/plans/2026-08-21-issue-1060-app-password-revocation-e2e.md`

**Interfaces:**

- Consumes:
  - `signInAsNewUser(page): Promise<string>` from `end2end/tests/helpers.ts`
  - `goto(page, path): Promise<void>` from `end2end/tests/helpers.ts`
  - `click(page, selector): Promise<void>` from `end2end/tests/helpers.ts`
  - `BASE_URL` from `end2end/tests/helpers.ts`
  - Playwright `request` fixture from `end2end/tests/fixtures.ts`
  - Existing local helper `mintAppPassword(page, label): Promise<string>`
- Produces:
  - A Playwright test named
    `an app password can be revoked from the sessions page` in
    `end2end/tests/atompub.spec.ts`
  - Updated App password management evidence in
    `docs/coverage/csr-e2e-matrix.md`

- [x] **Step 1: Write the failing e2e test**

  Add this test to `end2end/tests/atompub.spec.ts`, near the existing App
  Password tests:

  ```ts
  test("an app password can be revoked from the sessions page", async ({
    page,
    request,
  }) => {
    const username = await signInAsNewUser(page);
    const label = "Revoked App Password e2e";

    const token = await mintAppPassword(page, label);
    const appPasswordRow = page.locator("li", { hasText: label });
    await expect(appPasswordRow).toBeVisible();

    await click(page, `li:has-text("${label}") button:has-text("Revoke")`);

    await expect(appPasswordRow).toHaveCount(0);
    await expect(page.locator("li", { hasText: "(current)" })).toBeVisible();

    const auth =
      "Basic " + Buffer.from(`${username}:${token}`).toString("base64");
    const response = await request.get(`${BASE_URL}/atompub/service`, {
      headers: { authorization: auth },
    });
    expect(response.ok()).toBeFalsy();
    expect(response.status()).toBe(401);
  });
  ```

  Keep `request`, not `page.request`; the test's proof depends on avoiding the
  browser cookie path.

- [x] **Step 2: Run the targeted spec and verify the new test fails before any
      implementation repair**

  Run:

  ```bash
  devtool run -- cargo xtask e2e-local atompub.spec.ts
  ```

  Expected: FAIL only if the current UI/server behavior does not already satisfy
  the new coverage contract. If it passes immediately, record that the issue was
  a pure coverage gap and continue; the new test itself is still the behavior
  lock.

  Evidence: the sandboxed run failed before Playwright because the local server
  could not start (`.xtask/run/1787316376798-2.out`). The escalated run reached
  Playwright and failed only on the borrowed `"unauthorized"` body assertion
  (`.xtask/run/1787316581123-1595900.out`): revocation itself produced a non-OK
  AtomPub response with an empty body, matching the AtomPub route's 401
  projection rather than server-fn error serialization.

- [x] **Step 3: Repair only if the test exposes a product bug**

  If Step 2 fails for an implementation reason, make the smallest owning change:
  keep revocation in `web/src/sessions/api.rs` / `web/src/sessions/component.rs`
  or the AtomPub auth surface that actually failed. Do not introduce a new App
  Password model, direct endpoint shortcut, or broad UI refactor.

  If Step 2 passes, make no product-code change.

- [x] **Step 4: Update the CSR coverage matrix**

  In `docs/coverage/csr-e2e-matrix.md`, update the App password management
  section so the covered evidence names create, display, and revoke coverage in
  `end2end/tests/atompub.spec.ts`, and remove the stale "Uncovered revoke
  behavior" paragraph for #1060.

- [x] **Step 5: Run the targeted spec and verify it passes**

  Run:

  ```bash
  devtool run -- cargo xtask e2e-local atompub.spec.ts
  ```

  Expected: PASS.

  Evidence: the targeted escalated rerun passed:
  `.xtask/run/1787316739907-1618992.out`.

- [x] **Step 6: Tick this task, run the per-commit gate, and commit**

  First change this task's checkbox to `- [x]`. Then run:

  ```bash
  devtool run -- cargo xtask check
  ```

  Expected: PASS, with only intentional formatter or coverage-doc updates.
  Inspect `git status --short` and stage exactly the checked work:

  Evidence: the sandboxed check failed on host cache permissions only
  (`.xtask/run/1787316869731-2.out`). The escalated full check passed:
  `.xtask/run/1787316922017-1637581.out`.

  ```bash
  git add end2end/tests/atompub.spec.ts docs/coverage/csr-e2e-matrix.md docs/superpowers/specs/2026-08-21-issue-1060-app-password-revocation-e2e.md docs/superpowers/plans/2026-08-21-issue-1060-app-password-revocation-e2e.md
  git commit -m "test(e2e): cover app-password revocation"
  ```

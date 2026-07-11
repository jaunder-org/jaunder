# Plan — #347: manual refresh for the subscriber roster

**Spec:**
[`docs/superpowers/specs/2026-07-11-issue-347-roster-manual-refresh.md`](../specs/2026-07-11-issue-347-roster-manual-refresh.md)
**Issue:** [#347](https://github.com/jaunder-org/jaunder/issues/347) ·
**Branch/worktree:** `worktree-issue-347-roster-refetch` **For agentic
workers:** drive with **jaunder-iterate**; delegate a task via
**jaunder-dispatch** if useful. Tick checkboxes live.

## Review header

**Goal.** Make `AudiencesPage`'s once-fetched subscriber roster refreshable
in-page: an `Invalidator`+`sticky` roster plus a refresh icon in the "Your
audiences" card head, so a mid-session new subscriber appears without a full
reload.

**Scope.**

- **In:** `web/src/audiences/mod.rs` (redefine `RosterSignal`; roster →
  `Invalidator::sticky`; refresh `<button>`), `web/src/render/mod.rs`
  (`Icons::REFRESH`), `server/assets/jaunder.css` (`.j-icon-btn`),
  `end2end/tests/audiences.spec.ts` (refresh test).
- **Out:** live/push updates; refetch-on-focus (rejected); any change to the
  audience-list/members resources (#348/#359/#372) or #346's error-node shape.
  No new ADR, no coverage markers (all touched Rust is `#[component]` / a
  `const`, exempt per ADR-0050).

**Tasks.**

1. e2e refresh test — RED (no control yet). _(uncommitted — a red e2e would
   break the shared gate)_
2. Implement (glyph + alias + `sticky` roster + button + CSS) — GREEN, gate
   clean, commit test + impl together.

**No separable concerns** surfaced; nothing to file.

**Key risks / decisions.**

- **`sticky` stringifies the error** → `RosterSignal` is redefined to a `String`
  error (behavior-transparent; the node `Display`s it, the checklist ignores
  it). Aligns with the members path.
- **`Invalidator` is `Copy`** (its own `#[derive(Clone, Copy, Debug)]`,
  `reactive.rs`), so the bare `roster` can feed both `sticky(&self)` and
  `move |_| roster.notify()`.
- **Icon glyph is visual** — the e2e finds the button by role+name, not shape,
  so the test can't catch a wrong-looking glyph; **eyeball `Icons::REFRESH` in
  the running app** during Task 2 (the `/run` verify step).
- **Import paths** for `Icon`/`Icons` into `audiences/mod.rs` must be confirmed
  at edit time.

## Global constraints

- Run from the worktree. Fast gate:
  `devtool run -- cargo xtask check --no-test`; full/commit gate:
  `devtool run -- cargo xtask check`. E2e:
  `devtool run -- cargo xtask e2e-local audiences.spec.ts` while iterating; full
  matrix at ship.
- **Commit only green.** Task 1's red e2e stays local until Task 2 makes it
  pass.
- `web` is dual-target (host + wasm) — clippy runs both. No `Co-Authored-By`.
  Follow `CONTRIBUTING.md`.
- **leptosfmt hazard:** keep intent comments _outside_ `view!` and away from
  `return` (they get relocated). Put "why" on the wiring above the macro.

---

## Task 1 — e2e: mid-session refresh test (RED)

**File:** `end2end/tests/audiences.spec.ts` (append one test; reuse `register`,
`subscribeTo`, `goto`, `click` from `./helpers`).

```ts
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
  // Roster fetched empty at load: the checklist shows the empty message, no candidates.
  await expect(friends.getByText("No active subscribers yet.")).toBeVisible();

  // A subscriber arrives mid-session (another user's session).
  const xCtx = await browser.newContext();
  const xPage = await xCtx.newPage();
  const userX = await register(xPage, firstNav);
  await subscribeTo(xPage, author);
  await xCtx.close();

  // The once-fetched roster hasn't updated — X is not yet a candidate.
  await expect(
    friends.locator(".j-audience-members li").filter({ hasText: userX }),
  ).toHaveCount(0);

  // Click the refresh control (by accessible name), then X appears as an "Add" candidate — no reload.
  await page.getByRole("button", { name: "Refresh subscribers" }).click();
  const friendsX = friends
    .locator(".j-audience-members li")
    .filter({ hasText: userX });
  await expect(friendsX.locator('button:has-text("Add")')).toBeVisible();
});
```

**Run:** `devtool run -- cargo xtask e2e-local audiences.spec.ts`. **Expected:**
RED — `getByRole("button", { name: "Refresh subscribers" })` finds nothing
pre-implementation, so `.click()` times out. **Do NOT commit.**

---

## Task 2 — implement the manual refresh (GREEN) + commit

**2a — `Icons::REFRESH` glyph** (`web/src/render/mod.rs`, in the `Icons` impl
~line 559). Add a circular-arrow glyph in the `0 0 20 20` viewBox convention,
e.g.:

```rust
pub const REFRESH: &'static str = "M15.5 8A6 6 0 1 0 16 11 M15.5 4v4.5h-4.5";
```

Candidate path — **visually confirm** in the running app (Task 2 verify) and
adjust the arc/arrowhead until it reads as "refresh"; the e2e won't catch a bad
shape.

**2b — redefine `RosterSignal`** (`web/src/audiences/mod.rs:61`) to the
`String`-error shape `sticky` returns:

```rust
type RosterSignal = Signal<Option<Result<Vec<SubscriberSummary>, String>>>;
```

Keep/adjust its doc comment (still "full resolved state"; error is now
`String`). `WebResult` stays imported (the server fn still returns it).

**2c — roster → `Invalidator::sticky` + refresh button**
(`web/src/audiences/mod.rs`). Replace the roster wiring (`mod.rs:262-268`):

```rust
    // The subscriber roster: an `Invalidator`-driven `sticky` resource so the refresh
    // control refetches it while retaining the current roster (flash-free). Provided as a
    // `RosterSignal` — one source of truth for the page-level error node below and each
    // `MemberChecklist`. A fetch error is surfaced, never swallowed into an empty roster (#346).
    let roster = Invalidator::new();
    let subscribers: RosterSignal = roster.sticky(|| list_my_subscribers());
    provide_context(subscribers);
```

Add the control inside the `j-card-head` (after the title `</div>`,
`mod.rs:283`), so flex + `margin-left:auto` right-aligns it:

```rust
                        // Manual roster refresh (#347): the roster is fetched once at load,
                        // so a mid-session new subscriber needs this to appear without a reload.
                        <button
                            type="button"
                            class="j-icon-btn"
                            aria-label="Refresh subscribers"
                            on:click=move |_| roster.notify()
                        >
                            <Icon path=Icons::REFRESH />
                        </button>
```

Add imports at the top of `audiences/mod.rs` — `Icon` (confirm:
`crate::ui::Icon` or `crate::pages::ui::Icon`) and `Icons`
(`crate::render::Icons`). The page-level error node (`mod.rs:288-299`) and
`MemberChecklist` are **unchanged**.

**2d — `.j-icon-btn` CSS** (`server/assets/jaunder.css`, near
`.j-tag-chip-remove` ~line 616). A minimal borderless icon button + right-align
in a card head:

```css
.j-icon-btn {
  background: none;
  border: none;
  cursor: pointer;
  color: var(--muted-soft);
  padding: 0;
  display: inline-flex;
  align-items: center;
}
.j-icon-btn:hover {
  color: var(--ink);
}
.j-card-head .j-icon-btn {
  margin-left: auto;
}
```

**Verify:**

- `devtool run -- cargo xtask check` — clean (fmt, leptosfmt, prettier for the
  CSS, host + wasm clippy, coverage/tests). No new coverage markers (all edits
  are `#[component]` bodies, a `const`, or CSS).
- `devtool run -- cargo xtask e2e-local audiences.spec.ts` — Task 1 test
  **GREEN**; the existing CRUD + #346/#383 error-branch tests still **GREEN**.
- **`/run` the app** and eyeball the refresh icon in the audiences card head
  (glyph reads as "refresh", right-aligned, hover state), and drive the refresh
  once by hand.

**Commit** (per **jaunder-commit**, after `check` is clean — one commit for
impl + test):

```
web(audiences): manual refresh for the subscriber roster (#347)
```

---

## Self-review checklist

- [ ] `RosterSignal` is the `String`-error shape; roster is
      `roster.sticky(|| list_my_subscribers())`; `provide_context` unchanged.
- [ ] Refresh `<button aria-label="Refresh subscribers" class="j-icon-btn">` in
      the card head, `on:click` → `roster.notify()`, wrapping
      `<Icon path=Icons::REFRESH/>`.
- [ ] `Icons::REFRESH` added and **visually confirmed**; `Icon`/`Icons`
      imported.
- [ ] `.j-icon-btn` CSS added, right-aligned in the card head; prettier-clean.
- [ ] #346's error node + `MemberChecklist` untouched; error now `String` (node
      `Display`s it).
- [ ] E2e: create audience → empty message → X subscribes → X absent → click
      refresh → X is an "Add" candidate. Green; every commit green; no
      `Co-Authored-By`.
- [ ] No change to list/members resources (#348/#359/#372), #346's shape, or new
      ADR.

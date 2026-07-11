# Plan — #346: surface a `list_my_subscribers` roster fetch error

**Spec:**
[`docs/superpowers/specs/2026-07-11-issue-346-roster-error-surfacing.md`](../specs/2026-07-11-issue-346-roster-error-surfacing.md)
**Issue:** [#346](https://github.com/jaunder-org/jaunder/issues/346) ·
**Branch/worktree:** `worktree-issue-346-roster-error` (rebased onto post-#381
`main`) **For agentic workers:** drive with **jaunder-iterate**; a task may be
delegated via **jaunder-dispatch**. Tick checkboxes live.

## Review header

**Goal.** Stop `AudiencesPage` swallowing a `list_my_subscribers` fetch error
into an empty roster: carry the roster's full resolved state through context
and, on error, render one page-level `p.error` while checklists suppress the
misleading "No active subscribers yet.".

**Scope.**

- **In:** `web/src/audiences/mod.rs` (roster context type + page-level error
  node + `MemberChecklist` gating); `end2end/tests/audiences.spec.ts` (error
  test via route interception + a genuine-empty regression guard).
- **Out:** members swallow (#381, merged); other verticals' error branches
  (#383); roster refetch (#347). No new ADR, no coverage markers (all touched
  code is `#[component]`, exempt per ADR-0050).

**Tasks.**

1. Write the e2e error test (route interception) + genuine-empty guard — observe
   RED, prove the interception mechanism. _(not committed — a red e2e would
   break the shared gate)_
2. Implement the Rust change (roster context `Option<WebResult<Vec>>` +
   page-level error node + checklist gating) — turn the test GREEN, gate clean,
   commit test + impl together.

**Separable concerns:** already filed — **#383** (generalize fault injection),
`blocked-by #346`. No filing task remains.

**Key risks / decisions.**

- **Fault-injection pattern is unproven in-repo.** Task 1 de-risks it: a forced
  500 must deterministically reproduce the bug (real subscriber → "No active
  subscribers yet."). If that symptom doesn't appear, the interception isn't
  driving `Err` — adjust the `fulfill` shape (body/status) before proceeding.
  Works because the app is CSR (real client `POST`).
- **Representation:** `Signal<Option<WebResult<Vec<SubscriberSummary>>>>`
  (single source of truth; mirrors #381's `member_ids`). `WebError: Clone` holds
  (it lives in `sticky`'s signal).
- **`p.error` is shared** by the list-error and create-form nodes — assertions
  pin the "Couldn't load your subscribers" substring, never a bare `p.error`
  count.

## Global constraints

- Run everything from the worktree. Gate: `devtool run -- cargo xtask check`
  (fmt + clippy over **both** host and wasm targets + Nix coverage/tests). E2e:
  `devtool run -- cargo xtask e2e sqlite chromium` while iterating; the full
  `{sqlite,postgres}×{chromium,firefox}` matrix runs at `validate`/ship.
- **Commit only green.** The e2e gate runs in `validate`/CI; never commit a
  failing e2e. Task 1's red test stays local until Task 2 makes it pass.
- No `Co-Authored-By` trailer. Follow `CONTRIBUTING.md`.

---

## Task 1 — e2e: roster-error test (RED) + genuine-empty guard

**Files:** `end2end/tests/audiences.spec.ts` (append two tests; reuse
`register`, `subscribeTo`, `goto`, `click` from `./helpers` and `setTestBudget`,
`slowBrowserFirstNavigationTimeoutMs` from `./fixtures`).

**Test A — error surfaces, not an empty roster (AC1, AC2).**

```ts
test("Audiences: a failed subscriber-roster fetch surfaces an error, not an empty roster", async ({
  page,
  browser,
}, testInfo) => {
  setTestBudget(120_000);
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  const author = await register(page, firstNav);

  // A real subscriber, so an empty roster would be a lie (this is the bug).
  // Mirrors the existing test's setup (audiences.spec.ts:36-40): subscribeTo(page, authorUsername).
  const xCtx = await browser.newContext();
  const xPage = await xCtx.newPage();
  await register(xPage, firstNav);
  await subscribeTo(xPage, author);
  await xCtx.close();

  // Force the roster fetch to fail. Substring/regex match per the suite's url.includes convention.
  await page.route(/\/api\/list_my_subscribers/, (route) =>
    route.fulfill({ status: 500, body: "boom" }),
  );

  await goto(page, "/audiences");

  // AC1 (zero-audience path): the page-level roster error shows before any audience exists.
  await expect(page.getByText("Couldn't load your subscribers")).toBeVisible();

  // With an audience present, a checklist mounts — the empty-roster lie must NOT appear.
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  await expect(
    page.locator(".j-audience-item", { hasText: "Friends" }),
  ).toBeVisible();

  // AC2: no "No active subscribers yet." lie, and no add/remove list rendered.
  await expect(page.getByText("No active subscribers yet")).toHaveCount(0);
  await expect(page.locator(".j-audience-members")).toHaveCount(0);
});
```

**Test B — a genuine empty roster still says so (AC3 regression guard).** Author
with an audience but **no** subscribers, no interception:

```ts
test("Audiences: a genuinely empty roster still shows the empty message", async ({
  page,
}, testInfo) => {
  setTestBudget(120_000);
  const firstNav = slowBrowserFirstNavigationTimeoutMs(testInfo, 20_000);
  await register(page, firstNav);
  await goto(page, "/audiences");
  await page.fill('input[placeholder="Audience name"]', "Friends");
  await click(page, 'button:has-text("Create")');
  await expect(page.getByText("No active subscribers yet")).toBeVisible();
  await expect(page.getByText("Couldn't load your subscribers")).toHaveCount(0);
});
```

**Run:** `devtool run -- cargo xtask e2e sqlite chromium` (optionally Playwright
`--grep` on the titles). **Expected:** Test A **RED** (current code has no
"Couldn't load your subscribers" string; the intercepted 500 → empty roster →
"No active subscribers yet." shows — this reproduces the bug and proves the
interception drives `Err`). Test B **GREEN** pre-fix (characterization guard).
**De-risk gate:** if Test A does _not_ show "No active subscribers yet." under
the 500 (e.g., stuck on a members/roster loader, or the real roster loads
anyway), the interception isn't matching / not driving `Err` — fix the route
pattern or `fulfill` before Task 2. **Do NOT commit** (Test A is red).

> Note: confirm the exact `register`/`subscribeTo` helper signatures against the
> existing `audiences.spec.ts` setup (lines ~33–40) at implementation time;
> mirror them rather than the sketch above.

---

## Task 2 — implement the roster-error surfacing (GREEN) + commit

**Files:** `web/src/audiences/mod.rs`.

**2a — roster context carries the full resolved state** (replace
`mod.rs:260-266`):

```rust
let subscribers_res = crate::server_resource(|| (), |()| list_my_subscribers());
// Carry the full resolved state (loading `None` / `Some(Err)` / `Some(Ok)`) so the
// page-level error node and every `MemberChecklist` read one source of truth — the
// same shape #381 gave the members path (`member_ids`).
let subscribers: Signal<Option<WebResult<Vec<SubscriberSummary>>>> =
    Signal::derive(move || subscribers_res.get());
provide_context(subscribers);
```

Update the doc comment on `mod.rs:256-258` (it says the roster is "derived
straight from the resource … reflects it reactively when it resolves") to note
it now carries the error too.

**2b — page-level error node** (sibling in the "Your audiences" card, inserted
after the `j-card-head` `</div>` at `mod.rs:282`, before
`<ul class="j-audience-list">`):

```rust
// Roster fetch error: surfaced once here (the roster feeds every checklist), mirroring
// the audience-list error sibling below. Silent while loading / on success.
{move || {
    subscribers.get().and_then(Result::err).map(|e| {
        view! { <p class="error">{format!("Couldn't load your subscribers: {e}")}</p> }
    })
}}
```

**2c — `MemberChecklist` gating.** Change the context read (`mod.rs:389`) to
`expect_context::<Signal<Option<WebResult<Vec<SubscriberSummary>>>>>()`, and
inside the existing `Some(Ok(member_ids)) =>` arm (`mod.rs:410`), replace the
`if subscribers.is_empty()` branch (`mod.rs:411`) with a match over the roster
state (`subscribers` is now `Option<WebResult<Vec<SubscriberSummary>>>`):

```rust
Some(Ok(member_ids)) => {
    match subscribers {
        // Loading or errored: render nothing — the page-level node shows the error.
        None | Some(Err(_)) => ().into_any(),
        // Genuinely empty roster: unchanged message.
        Some(Ok(subs)) if subs.is_empty() => {
            view! { <p class="j-sub">"No active subscribers yet."</p> }.into_any()
        }
        // Loaded: the add/remove list (existing body, now bound from `subs`).
        Some(Ok(subs)) => {
            view! {
                <ul class="j-audience-members">
                    {subs.into_iter().map(|sub| { /* existing per-subscriber add/remove view */ }).collect::<Vec<_>>()}
                </ul>
            }
            .into_any()
        }
    }
}
```

Keep the existing `let subscribers = subscribers.get();` read at the top of the
render closure (`mod.rs:400`); it now yields `Option<WebResult<Vec<_>>>`. Ensure
`WebResult` is in scope (already imported: `use crate::error::WebResult;`,
`mod.rs:17`).

**Verify:**

- `devtool run -- cargo xtask check` — clean (fmt, host + wasm clippy,
  coverage/tests). No new coverage markers expected (all edits are
  `#[component]` bodies).
- `devtool run -- cargo xtask e2e sqlite chromium` — Task 1 Test A + Test B
  **GREEN**; the existing "CRUD + membership toggle" test still **GREEN** (AC4
  happy path preserved).
- Manual `/verify`-style drive: load `/audiences` normally (roster loads, Add
  buttons show), then with the temporary route intercept (error node +
  suppression).

**Commit** (per **jaunder-commit**, after `check` is clean — one coherent commit
for impl + tests):

```
web(audiences): surface list_my_subscribers roster fetch error (#346)
```

---

## Self-review checklist

- [ ] Roster context is `Signal<Option<WebResult<Vec<SubscriberSummary>>>>`;
      derive is the pass-through; `provide_context` + `expect_context` types
      match.
- [ ] Page-level `p.error` reads the roster signal, static "Couldn't load your
      subscribers: " prefix, in the audiences card.
- [ ] Checklist: `None | Some(Err(_))` ⇒ nothing; `Some(Ok(empty))` ⇒ "No active
      subscribers yet."; `Some(Ok(list))` ⇒ add/remove list. Nested inside the
      `Some(Ok(member_ids))` members arm.
- [ ] E2e: error test pins the "Couldn't load your subscribers" substring (not a
      bare `p.error`); genuine-empty guard present; happy-path test untouched.
- [ ] Gate green; every commit green; no `Co-Authored-By`.
- [ ] No edits to members wiring (#381), other verticals (#383), or roster
      refetch (#347).

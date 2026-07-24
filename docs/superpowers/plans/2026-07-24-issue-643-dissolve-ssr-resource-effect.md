# Dissolve SSR-era Resource→Effect→signal indirections (posts) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
`docs/superpowers/specs/2026-07-24-issue-643-dissolve-ssr-resource-effect.md`
(decisions D1–D7, acceptance AC1–AC10). This plan is "how"; the spec is
"what/why". Read the spec's Decisions section before starting a task.

**Goal:** Replace the four SSR-era `Resource→Effect→signal` copies in
`web/src/posts/component.rs` with their CSR-native shapes, converge the user
timeline onto the shared `TimelineState`/`TimelineRows` bundle, and retire the
dead SSR doctrine that authored them.

**Architecture:** All four sites live in one wasm-only view file (ADR-0070: view
fns are e2e-tested, pure logic is host-tested in `state.rs`/`render.rs`). Three
of the changes are view-only (no host-unit-testable seam) and are pinned by
e2e + the wasm-clippy/validate gate; the fourth (site 3) reuses the
already-host-tested `timeline::state` model.

**Tech Stack:** Rust, Leptos (CSR), `cargo`/`nextest`, Playwright (`end2end`).

## Global Constraints

_Every task's requirements implicitly include these._

- **No `Co-Authored-By` trailer** on any commit (global CLAUDE.md).
- **Coverage policy:** no new `cov:ignore` without justification; CRAP threshold
  respected (AC10). No new host-coverable logic is introduced — the changes are
  view code + a pure-model reuse.
- **Wasm clippy before commit** for view changes:
  `cargo clippy -p web --target wasm32 -- -D warnings` (a plain `cargo check`
  skips it and the slow gate then fails on `must_use_candidate` etc.).
- **Per-commit gate:** `cargo xtask check` (fmt + clippy + Nix coverage/tests)
  runs in the pre-commit hook — run it first so it passes clean
  (`jaunder-commit`). Serialize edit→gate→commit (Nix builds the working tree
  mid-commit).
- **Behavior identical** (AC8): no functional change to any flow; the user
  timeline's load-more mechanism moves `ServerAction`→`spawn_local` but its
  observable behavior (initial load, pagination, error, empty) is preserved.
- **`leptosfmt`/rustfmt** may relocate comments in `view!`; keep intent comments
  outside the macro.
- **Line numbers are pre-edit anchors.** Tasks 4–7 all edit
  `web/src/posts/component.rs` sequentially, so absolute line numbers cited in
  later tasks drift as earlier tasks delete lines. Relocate each edit by its
  function name + the quoted code/grep anchor in the step, not the line number.

---

## Review header

**Scope — in:**

- The four sites in `web/src/posts/component.rs` (D1–D3).
- Additive widening of `web/src/timeline/component.rs` `TimelineRows` (D4).
- Narrow rewrite of `docs/web-style-guide.md` §9 (D5).
- Stale `pages::ui` module-doc fix in `web/src/posts/render.rs` (D6; the
  `render/mod.rs` refs turned out valid — see D6/Task 8).
- One new characterization e2e for the edit-page audience seed (D7, revised —
  see Key risks).

**Scope — out (filed as Task 1):**

- LogoutPage never-seen "logged out" message (auth vertical, own verification).
- Any projector-seed (`PageSeed`/`state.adopt`) mechanics change — current
  design, explicitly not a remnant.

**Tasks:**

1. File the LogoutPage split-out issue (`jaunder-issues`). → **filed as #649**.
2. Characterization e2e: edit page pre-selects the post's current audience —
   confirm GREEN on current code (pins existing behavior before we refactor).
3. Widen `TimelineRows` with an optional `tag_context` prop (additive).
4. Site 1 — `AudiencePicker`: dissolve `named_audiences` signal + Effect (D1).
5. Site 2 — `PostCreateForm`: rewrite the dead SSR comment to a CSR reason;
   Effect preserved (D2).
6. Site 4 — `EditPostPage`: fold the `current_audience` seed into the existing
   `Suspense`/`Suspend` block; dissolve the Effect (D2).
7. Site 3 — `UserTimelinePage`: converge onto `TimelineState` +
   `spawn_load_more` + `TimelineRows`; keep one loading-gate signal (D3).
8. Narrow §9 rewrite + `pages::ui` doc sweep (D5, D6).
9. Final verification: `validate --no-e2e` + wasm clippy + `e2e-local` on
   posts/visibility/audiences (AC8).

**Key risks / decisions:**

- **AC9 amendment (needs a nod at plan approval).** The spec's AC9 lists two new
  e2e assertions. **Proposed: drop the create-default-audience one, keep the
  edit-current-audience one (Task 2).** The create-default e2e is _technically
  feasible_ — the site default is settable for e2e via
  `jaunder site-config set posts.default_audience <base>` (CLI at
  `server/src/cli.rs:236–258`, key `posts.default_audience` at :245; the e2e
  harness seeds config through the shipped binary the same way `feeds.spec.ts`
  seeds `feeds.websub_hub_url`). But it isn't worth it: (a)
  `posts.default_audience` is **global server-boot config**, so flipping it
  non-`Public` on the shared e2e server would contaminate every other
  create/publish test that assumes the `Public` default — isolating it would
  need a dedicated bespoke-server spec; and (b) decisively, **site 2 is
  comment-only under D2** (the Effect is preserved), so this e2e would guard no
  actual code change. The edit-current e2e, by contrast, guards a real code
  change (D2 site 4, Effect→`Suspense`) via existing UI. Site 2's seed wiring
  stays indirectly exercised by every create test (the composer renders its
  picker) and its server-fn (`get_default_audience`) is already host-tested
  (`storage/src/site_config.rs`, the `default_audience_*` cases).
- **Site 3 load-more mechanism change** (`ServerAction`→`spawn_local`): covered
  by `posts.spec.ts:356` (pagination). Verify no double-fetch / lost cursor.
- **Site 3 loading gate:** the shared `TimelineState`/`LoadStatus` has no
  "never-loaded" state, so a single `loaded` bool survives to keep the
  "Loading…" placeholder on the unseeded client-nav path (AC8) instead of
  flashing `TimelineRows`' "No posts yet." empty state.
- **§9 anchor stability:** ADR-0061 cites "web-style-guide §9" (×2) for the
  sticky-copy subsection — keep the section **number 9** and that subsection
  intact; rewrite only anti-pattern #1's SSR rationale.

---

## Task 1: File the LogoutPage split-out issue

**Files:** none (tracker only).

**Interfaces:**

- Produces: a GitHub issue number the ship step can reference; nothing later
  tasks consume.

- [x] **Step 1: Create the issue** via `jaunder-issues` (web vertical, Task
      type, milestone "Web: canonical Leptos CSR convergence"). Title e.g.
      _"auth: LogoutPage 'You have been logged out.' message is never seen
      (redirect races it)"_. Body: `web/src/auth/component.rs:110–128` renders
      the message from `logout_action.value()`, but the #591 pushState redirect
      hook navigates to `/` on the server fn's `redirect("/")`, so the message
      likely never paints. Confirm what a user actually sees; either remove the
      dead branch or keep the page honest. Reference #643 as the origin.

- [x] **Step 2: Record the number** in this plan's Review header (Task 1 line)
      so `jaunder-ship` can cross-link it. No commit (tracker-only task).

---

## Task 2: Characterization e2e — edit page pre-selects the current audience

Pins the site-4 seed behavior **before** Task 6 changes its mechanism. It must
pass on the current (unmodified) code — it is a characterization test.

**Files:**

- Modify: `end2end/tests/posts.spec.ts` (add one `test(...)`).
- Reference (reuse helpers, do not duplicate):
  `end2end/tests/visibility.spec.ts` (`publishWithBaseAudience`,
  `#audience-base` selection, named-audience checkbox flow at :201–249) and
  `end2end/tests/audiences.spec.ts` (creating a named audience).

**Interfaces:**

- Consumes: existing e2e fixtures (`registeredPage`, audience-creation +
  targeted-publish helpers).
- Produces: nothing later tasks import; it is a standing guard.

- [ ] **Step 1: Write the test.** Registered author creates a named audience,
      creates/publishes a post targeted to `Subscribers` **plus** that named
      audience, opens that post's **edit** page, and asserts the picker reflects
      the stored targeting:

```ts
test("edit page pre-selects the post's current audience", async ({
  registeredPage: page,
}) => {
  // 1. Create a named audience (reuse the audiences-page flow).
  // 2. Publish a post targeted Subscribers + <named>, capturing its edit URL
  //    (mirror visibility.spec.ts scenario 3's targeting steps).
  await page.goto(editUrl);
  await waitForSelector(page, "#audience-base");
  // Seed pre-selects the stored base:
  await expect(page.locator("#audience-base")).toHaveValue("subscribers");
  // ...and pre-checks the named-audience checkbox:
  await expect(
    page.locator(`#audience-named-${namedAudienceId}`),
  ).toBeChecked();
});
```

- [ ] **Step 2: Run it against current code, verify PASS.**

Run: `cargo xtask e2e-local posts` Expected: PASS (the current Effect-based seed
already produces this state) — proving the test captures real behavior, not the
refactor's.

- [ ] **Step 3: Commit.**

```bash
git add end2end/tests/posts.spec.ts
git commit -m "test(e2e): pin edit-page current-audience pre-selection"
```

---

## Task 3: Widen `TimelineRows` with an optional `tag_context`

Prerequisite for Task 7 (the user timeline needs `ForUser` tag links, which the
shared component does not currently pass). Additive: `home.rs`/`cockpit.rs` call
sites stay textually unchanged (AC5).

**Files:**

- Modify: `web/src/timeline/component.rs` (`TimelineRows` signature + the
  `PostCard` call at :115; add a `TagContext` import).

**Interfaces:**

- Consumes: `crate::render::TagCtx as TagContext` (an
  `enum { SiteWide, ForUser(Username) }`, derives `Clone`).
- Produces: `TimelineRows` now accepts
  `#[prop(default = TagContext::SiteWide)] tag_context: TagContext`, forwarded
  to each `PostCard` as `tag_context=tag_context.clone()`.

- [ ] **Step 1: Add the prop + forward it.** In `TimelineRows`
      (`web/src/timeline/component.rs:98`):

```rust
use crate::render::TagCtx as TagContext;
// ...
#[component]
pub fn TimelineRows(
    state: TimelineState,
    on_mutate: Callback<()>,
    on_load_more: Callback<()>,
    #[prop(default = TagContext::SiteWide)] tag_context: TagContext,
) -> impl IntoView {
```

In the rows map (`:114–116`), clone per row:

```rust
rows.into_iter()
    .map(|p| {
        view! {
            <PostCard
                post=p
                banner=None
                tag_context=tag_context.clone()
                on_mutate=on_mutate
            />
        }
    })
    .collect::<Vec<_>>()
    .into_any()
```

(`tag_context` is captured by the outer closure; per-row `.clone()` is required
because `PostCard` takes it by value. The mapping closure that reads
`read_rows()` re-runs on `rows` change — `tag_context` is `Clone` and stable, so
cloning is correct.)

- [ ] **Step 2: Verify the shared call sites are untouched.**

Run:
`rg -n "TimelineRows" web/src/home/component.rs web/src/cockpit/component.rs`
Expected: both still call `<TimelineRows state=… on_mutate=… on_load_more=… />`
with **no** `tag_context` (default `SiteWide` preserves current behavior).

- [ ] **Step 3: Gate.**

Run: `cargo clippy -p web --target wasm32 -- -D warnings` Then:
`cargo xtask check --no-test` Expected: clean (compiles; `home`/`cockpit`
unaffected).

- [ ] **Step 4: Commit.**

```bash
git add web/src/timeline/component.rs
git commit -m "feat(web/timeline): TimelineRows takes an optional tag_context"
```

---

## Task 4: Site 1 — dissolve `AudiencePicker`'s `named_audiences` copy (D1)

**Files:**

- Modify: `web/src/posts/component.rs:344–357` (declaration) and the checkbox
  view closure at `:396–413`.

**Interfaces:**

- Consumes: `named` `Resource<Option<Result<Vec<AudienceSummary>, WebError>>>`
  (from `list_my_audiences()`), `audience_checkbox(a, selection)`.
- Produces: no external interface change (`AudiencePicker` prop is unchanged).

- [ ] **Step 1: Delete the signal + Effect; read the Resource directly.** Remove
      `named_audiences` (`:352`) and the `Effect::new` (`:353–357`), and remove
      the SSR comment (`:345–350`). Keep the `named` `Resource`. In the view
      closure that currently reads `named_audiences.get()` (`:397`), read the
      resource:

```rust
{move || {
    let audiences = named.get().and_then(Result::ok).unwrap_or_default();
    if audiences.is_empty() {
        ().into_any()
    } else {
        let rows = audiences
            .into_iter()
            .map(|a| audience_checkbox(a, selection))
            .collect_view();
        view! {
            <div style="margin-top:8px">
                <span class="j-field-label">"Also share with"</span>
                {rows}
            </div>
        }
            .into_any()
    }
}}
```

(`named.get()` is `Option<Result<Vec<_>, _>>`; `.and_then(Result::ok)` folds
both "not yet resolved" and "resolved Err" to an empty list — identical
observable behavior to the old signal, which started empty and was only ever set
on `Some(Ok(_))`.)

- [ ] **Step 2: Gate.**

Run: `cargo clippy -p web --target wasm32 -- -D warnings` Expected: clean.
Confirm no `named_audiences`/`Effect` remain in the picker:
`rg -n "named_audiences|Effect::new" web/src/posts/component.rs` shows neither
in `AudiencePicker` (`:343–415`).

- [ ] **Step 3: Verify the named-audience flow in a browser.**

Run: `cargo xtask e2e-local visibility` Expected: PASS — Scenario 3 ("Named
audience…", :206) targets a named audience, which requires the checkbox to
render from the directly-read resource.

- [ ] **Step 4: Commit.**

```bash
git add web/src/posts/component.rs
git commit -m "refactor(web/posts): AudiencePicker reads the audiences Resource directly"
```

---

## Task 5: Site 2 — rewrite `PostCreateForm`'s dead SSR comment (D2)

Lowest-risk site: the `Effect` is genuinely needed (seed-then-edit; the composer
renders instantly and cannot wait on the async site default), so **only the
comment changes**.

**Files:**

- Modify: `web/src/posts/component.rs:489–493` (the comment above the seed
  `Effect`).

**Interfaces:** none change.

- [ ] **Step 1: Replace the SSR comment with the CSR reason.** Leave the
      `Effect` at `:493–497` intact; replace `:489–492`:

```rust
    // The site-wide default audience resolves asynchronously; the composer must
    // render immediately (no Suspense), so seed the editable `audience` signal
    // once the Resource resolves, over the Public placeholder above. The user
    // can then edit the selection via `AudiencePicker`.
    Effect::new(move |_| {
        if let Some(Ok(default)) = default_audience.get() {
            audience.set(default);
        }
    });
```

- [ ] **Step 2: Gate.**

Run: `cargo clippy -p web --target wasm32 -- -D warnings` Expected: clean.
Confirm no SSR language remains near the seed:
`rg -n "SSR|hydration|disposal|serialize|per-request" web/src/posts/component.rs`
returns nothing in `:460–540`.

- [ ] **Step 3: Commit.**

```bash
git add web/src/posts/component.rs
git commit -m "docs(web/posts): PostCreateForm audience seed comment states the CSR reason"
```

---

## Task 6: Site 4 — fold `EditPostPage`'s audience seed into `Suspense` (D2)

**Files:**

- Modify: `web/src/posts/component.rs:1586–1600` (delete the standalone Effect;
  keep the `current_audience` Resource) and the `Suspend` block at `:1607–1617`
  (seed `audience` inside it).

**Interfaces:**

- Consumes: `current_audience`
  `Resource<Option<Result<AudienceSelection, WebError>>>` (from
  `post_audience_selection(id)`), `audience: RwSignal<AudienceSelection>`.
- Produces: none change.

- [ ] **Step 1: Delete the seed Effect + its SSR comment** (`:1592–1600`). Keep
      the `current_audience` `Resource` declaration (`:1586–1591`).

- [ ] **Step 2: Seed inside the existing `Suspend` block.** In the async render
      at `:1607` (which already `await`s `post` and sets `body`/`format`/… from
      `fetched`), add — after the `post.await` `Ok(fetched)` arm sets the other
      fields — an await-and-seed for the current audience:

```rust
{move || Suspend::new(async move {
    match post.await {
        Ok(fetched) => {
            body.set(String::from(fetched.body.clone()));
            format.set(fetched.format);
            slug_field.value.set(fetched.slug.to_string());
            summary_field
                .value
                .set(fetched.summary.as_deref().unwrap_or_default().to_owned());
            post_tags.set(fetched.tags.clone());
            // Seed the audience picker with the post's stored targeting; on a
            // fetch error leave the Public default (matching the old Effect,
            // which only set on Ok). Awaited here so it resolves with the rest
            // of the form under the same Suspense.
            if let Ok(selection) = current_audience.await {
                audience.set(selection);
            }
            // ... existing dispatch_update + view! unchanged ...
        }
        Err(_) => { /* existing not-found arm unchanged */ }
    }
})}
```

(Folding `current_audience.await` into the same block makes the form wait for
both round-trips — both keyed on the same `post_id` — before first paint, and
drops the separate post-hydration Effect. On audience-fetch `Err` the Public
default survives, identical to the deleted Effect's `if let Some(Ok(...))`
guard.)

- [ ] **Step 3: Gate.**

Run: `cargo clippy -p web --target wasm32 -- -D warnings` Expected: clean.
Confirm the standalone seed Effect is gone:
`rg -n "current_audience" web/src/posts/component.rs` shows only the `Resource`
declaration and the in-`Suspend` `.await`, no `Effect::new`.

- [ ] **Step 4: Verify in a browser** — the Task 2 characterization test now
      exercises the refactored path.

Run: `cargo xtask e2e-local posts` Expected: PASS, including "edit page
pre-selects the post's current audience".

- [ ] **Step 5: Commit.**

```bash
git add web/src/posts/component.rs
git commit -m "refactor(web/posts): seed EditPostPage audience inside Suspense"
```

---

## Task 7: Site 3 — converge `UserTimelinePage` onto `TimelineState` (D3)

**Files:**

- Modify: `web/src/posts/component.rs:1347–1530` (`UserTimelinePage`). Add
  imports: `crate::timeline::{TimelineState, TimelineRows, spawn_load_more}`
  (confirm exact re-export paths via `web/src/timeline/mod.rs`), and reuse the
  existing `TagContext`, `PageSeed`, `list_user_posts` imports.

**Interfaces:**

- Consumes (Task 3):
  `TimelineRows(state, on_mutate, on_load_more, tag_context)`.
  `TimelineState::default()/adopt(page)/resolve(page)/fail(msg)`;
  `spawn_load_more(state, fetch)` where
  `fetch: FnOnce(Option<UtcInstant>, Option<PostId>, Option<PageSize>) -> Fut`;
  `list_user_posts(username, cursor_created_at, cursor_post_id, limit)`.
- Produces: none (component signature unchanged).

- [ ] **Step 1: Replace the state machine.** Delete the six ad-hoc signals
      (`timeline`, `next_cursor_created_at`, `next_cursor_post_id`, `has_more`,
      `error`, `initial_loaded`), the `load_more_action` `ServerAction`, its
      Effect (`:1426–1439`), the initial-page Effect's SSR comment
      (`:1399–1402`), and the inline row rendering (`:1481–1526`). Keep
      `username` (Memo), `mutate_version`, `on_mutate`, `initial_page`
      (Resource), the surrounding chrome (`FeedDiscovery`/`RsdDiscovery`/
      `Topbar`/`SubscribeButton`). Introduce:

```rust
let state = TimelineState::default();
let loaded = RwSignal::new(false); // CSR loading gate: the shared TimelineState
                                   // has no "never-loaded" state; without this,
                                   // the unseeded client-nav first load would
                                   // flash TimelineRows' "No posts yet." empty
                                   // state before the fetch resolves.

// Public projector seed (#178/#179): unchanged guard, now adopting into the
// shared bundle.
if let Some(PageSeed::Profile { username: seed_user, page }) =
    use_context::<Option<PageSeed>>().flatten()
{
    if username.get_untracked().as_ref() == Some(&seed_user) {
        state.adopt(page);
        loaded.set(true);
    }
}

// The initial-page fetch resolves on the client; seed the shared timeline once
// it arrives (load-more appends via spawn_load_more). Canonical CSR shape,
// mirroring home.rs / cockpit.rs.
Effect::new(move |_| {
    if let Some(result) = initial_page.try_get().flatten() {
        match result {
            Ok(page) => state.resolve(page),
            Err(err) => state.fail(err.to_string()),
        }
        loaded.set(true);
    }
});

let on_load_more = Callback::new(move |()| {
    let Some(u) = username.get_untracked() else { return };
    spawn_load_more(state, move |created_at, post_id, limit| {
        list_user_posts(u.clone(), created_at, post_id, limit)
    });
});

// Re-run the outer view closure only on a real failure transition, like home.rs.
let read_error = Memo::new(move |_| state.status.get().into_failure());
```

- [ ] **Step 2: Rebuild the inner view** — chrome unchanged; the content region
      gates on error → loaded → `TimelineRows`:

```rust
{move || {
    if let Some(err) = read_error.get() {
        return view! { <p class="error">{err}</p> }.into_any();
    }
    if !loaded.get() {
        return view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any();
    }
    match username.get() {
        Some(user) => view! {
            <TimelineRows
                state=state
                on_mutate=on_mutate
                on_load_more=on_load_more
                tag_context=TagContext::ForUser(user)
            />
        }.into_any(),
        // Invalid username never reaches here: initial_page resolves Err and the
        // error branch above renders instead. Kept total for the type checker.
        None => ().into_any(),
    }
}}
```

(The old `read_*` helper closures and the `display_username` heading logic are
preserved where the `Topbar` needs them; only the timeline body is replaced.)

- [ ] **Step 3: Gate.**

Run: `cargo clippy -p web --target wasm32 -- -D warnings` Then:
`cargo xtask check --no-test` Expected: clean. Confirm removal:
`rg -n "ServerAction::<ListUserPosts>|initial_loaded|next_cursor_created_at" web/src/posts/component.rs`
returns nothing in `UserTimelinePage`; `ListUserPosts` may still be imported iff
used elsewhere — if now unused, drop the import.

- [ ] **Step 4: Verify load + pagination in a browser; manually verify the
      no-flash gate.**

Run: `cargo xtask e2e-local posts` Expected: PASS — `:356` ("per-user timeline
lists published posts with pagination") covers initial load + load-more.

The AC8 no-flash gate (unseeded first load shows "Loading…", not "No posts
yet.") is verified **manually, not asserted** — a Playwright assertion on an
intermediate loading frame is inherently racy (the fetch can resolve before the
assertion runs). Observation: with network throttled (DevTools "Slow 3G" or a
Playwright route delay), perform a **client-side** navigation to a profile that
has posts (e.g. from the cockpit, not a fresh page load, so no projector seed),
and confirm the content region shows the "Loading…" placeholder before the rows
appear — never the "No posts yet." empty state. Record the observation in the
task completion note.

- [ ] **Step 5: Commit.**

```bash
git add web/src/posts/component.rs
git commit -m "refactor(web/posts): converge UserTimelinePage onto shared TimelineState"
```

---

## Task 8: Narrow §9 rewrite + `pages::ui` doc sweep (D5, D6)

**Files:**

- Modify: `docs/web-style-guide.md:225–264` (anti-pattern #1 rationale + #2's
  SSR framing; **preserve** `:266–279` sticky-copy subsection verbatim and the
  section number `9`).
- Modify: `web/src/posts/render.rs:5` (`pages::ui` → `posts::component`).
  **Corrected during execution:** `web/src/render/mod.rs:50,281,315` were _not_
  touched — those `pages::ui::Sidebar` / `pages::ui::TagContext` refs are valid
  current paths (`pages/ui.rs` defines `Sidebar` and re-exports `TagContext`),
  not stale. Only `PostDisplay`'s path had moved.

**Interfaces:** none (docs).

- [ ] **Step 1: Rewrite §9 narrowly.** Retitle
      `## 9. SSR-safe Resource     patterns` →
      `## 9. Resource → signal patterns (CSR)` (keep the digit `9`; ADR-0061
      cites "§9"). Replace anti-pattern #1's disposal-race/ serialization
      rationale (`:230–236`) with the CSR reality: routed Leptos components
      serve a static CSR shell and mount fresh via `mount_to_body` (no hydration
      — `csr/src/lib.rs:10–12,44`; the `server`/`leptos/ssr` feature serves
      server-fns, the projector's render fns, and `leptos_axum` routing, **not**
      component hydration), so a plain client-only `Effect::new` copying a
      resolved `Resource` into signals is the normal idiom. **Keep** the
      wasm-only-placement rule (`:238–241`) and the "mirror `home.rs`" guidance
      (`:262–264`). For anti-pattern #2 (`:243–260`), keep the substantive
      ADR-0016 handle-first / graceful-`Err` / read-context-before- `await`
      guidance; drop only the now-false SSR claims ("resolved during SSR",
      "serializes its value to the client and is not re-fetched on hydration").
      **Do not touch** `:266–279` (sticky-copy / `Invalidator::     sticky` /
      `MemberChecklist`).

- [ ] **Step 2: Sweep the stale doc ref.** Replace `pages::ui` with
      `posts::component` at `web/src/posts/render.rs:5` (`PostDisplay`'s current
      home). **Verify first** that the `web/src/render/mod.rs:50,281,315` refs
      are _not_ stale — `pages/ui.rs` really defines `Sidebar` and re-exports
      `TagContext`, so `pages::ui::Sidebar` / `pages::ui::TagContext` resolve;
      leave them.

- [ ] **Step 3: Verify AC6 + AC7.**

Run: `rg -n "pages::ui" web/src/posts/render.rs` Expected: nothing (the valid
`render/mod.rs` refs remain). Run:
`rg -n "web-style-guide|§9" web/src docs/adr/0061-web-keyed-list-reactive-store.md docs/README.md`
Expected: every surviving citation still resolves — `component.rs` (sticky at
:334; client-only-Effect at the EditPostPage redirect ~:1557 after Task 6's line
shift), ADR-0061 (×2, sticky subsection preserved), README. Confirm no repo
comment cites §9 for an **SSR-safety** reason. Run:
`rg -n "SSR|hydration|disposal|serialize|new_isomorphic" web/src/posts/component.rs`
Expected: nothing (AC1 — all four spans clean).

- [ ] **Step 4: Gate (docs + prettier).**

Run: `cargo xtask check --no-test` Expected: clean (pre-commit prettier may
reflow the Markdown; re-stage if so).

- [ ] **Step 5: Commit.**

```bash
git add docs/web-style-guide.md web/src/posts/render.rs
git commit -m "docs(web): retire SSR-era §9 rationale; fix stale pages::ui ref in posts/render"
```

---

## Task 9: Final verification (AC8, AC10)

**Files:** none (verification only).

- [ ] **Step 1: Static + coverage gate.**

Run: `cargo xtask validate --no-e2e` (foreground, `timeout: 600000`) Expected:
green — static, clippy, coverage (no new `cov:ignore`; CRAP OK).

- [ ] **Step 2: Wasm clippy for `web`.**

Run: `cargo clippy -p web --target wasm32 -- -D warnings` Expected: clean.

- [ ] **Step 3: Browser verification of every touched flow.**

Run: `cargo xtask e2e-local posts` Run: `cargo xtask e2e-local visibility` Run:
`cargo xtask e2e-local audiences` Expected: all green — create-post, edit-post
(seed + publish redirect), user timeline (load + pagination), named-audience
picker, and the new edit-seed characterization test. Watch the user-timeline
first paint for any new flash.

- [ ] **Step 4: No commit** (verification task). Report results to
      `jaunder-iterate`; hand off to `jaunder-ship`.

---

## Self-review

- **Spec coverage:** D1→T4; D2→T5 (site 2) + T6 (site 4); D3→T3 (prereq) + T7;
  D4→T3; D5→T8; D6→T8; D7→T2 (revised, see Key risks). AC1→T4/T5/T6/T7/T8 greps;
  AC2→T4; AC3→T6; AC4→T7; AC5→T3; AC6/AC7→T8; AC8→T9; AC9→T2 (amended: create
  bullet dropped, edit bullet kept — flagged for approval); AC10→T9.
- **Placeholders:** none — each view task carries the actual edit + exact greps
  - exact `e2e-local`/clippy commands as its contract (view fns have no
    host-unit seam under ADR-0070; e2e + gate are the contract).
- **Type consistency:** `TimelineState`/`spawn_load_more`/`TimelineRows`/
  `TagContext::ForUser(Username)`/`list_user_posts(username, cursor_created_at, cursor_post_id, limit)`/`LoadStatus::into_failure`
  used in T7 match their definitions (`timeline/component.rs`,
  `timeline/state.rs`, `posts/api/listing.rs`) and T3's new prop.

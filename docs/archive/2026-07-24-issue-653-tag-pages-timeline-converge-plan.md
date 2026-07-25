# Converge SiteTagPage/UserTagPage onto TimelineState (flush reconcile-up) — Implementation Plan

> **For agentic workers:** Execute task-by-task with `jaunder-iterate`
> (delegating to a subagent via `jaunder-dispatch` when useful). Steps use
> checkbox (`- [ ]`).

**Spec:**
`docs/superpowers/specs/2026-07-24-issue-653-tag-pages-timeline-converge.md`
(decisions D1–D10, acceptance AC1–AC11). This plan is "how"; the spec is
"what/why".

**Goal:** Converge `SiteTagPage`/`UserTagPage` onto the shared
`TimelineState`/`spawn_load_more`/`TimelineRows` bundle, and flatten the
projector's `render_timeline_page` to the same wrapper-free `j-scroll` structure
so the projector-seeded timeline family (home, cockpit, user timeline, both tag
pages) is one flush, coincident shape — which also fixes #643's user-timeline
gutter flash.

**Architecture:** The projector twin (`render.rs`) is host-compiled +
coverage-measured (TDD: test-first). The `TimelineRows` widening + tag-page
views are wasm-only (ADR-0070), verified by wasm clippy + e2e. Both sides of the
coincidence move together.

**Tech Stack:** Rust, Leptos (CSR), `cargo`/`nextest`, Playwright (`end2end`).

## Global Constraints

- **No `Co-Authored-By` trailer** (global CLAUDE.md).
- **Coverage:** no new `cov:ignore` without justification; CRAP respected. The
  one new host-logic change (`render_timeline_page`) stays covered by its
  updated test.
- **Wasm clippy before commit** for view changes:
  `cargo clippy -p web --target wasm32 -- -D warnings`.
- **Per-commit gate:** `cargo xtask check` (pre-commit hook) — run first so it
  passes clean.
- **`leptosfmt` relocates comments inside `view!`** — keep intent comments
  outside the macro.
- **Line numbers are pre-edit anchors**; relocate by symbol/quoted anchor.
- **Behavior identical** except the intended flush layout (tag pages lose the
  `j-page` gutter; empty copy preserved via `empty_text`).

---

## Review header

**Scope — in:** `TimelineRows` (`+empty_text` prop); `render_timeline_page` +
its one coincidence test (`render.rs`); `SiteTagPage`/`UserTagPage` conversion +
orphaned-import removal (`posts/component.rs`); tag-route e2e assertion
updates + a new `UserTagPage` smoke test (`posts.spec.ts`).

**Scope — out:** `UserTimelinePage` (no change — the projector flatten fixes its
coincidence for free); home `SiteTimeline` projector branch (already flush); the
`Permalink` branch (keeps its own `j-page`); `TimelineState`/`spawn_load_more`
internals; the tags vertical (#328).

**Tasks:**

1. `TimelineRows` — add optional `empty_text` prop (default "No posts yet.").
2. Flatten `render_timeline_page` to wrapper-free `j-scroll` (test-first) —
   fixes #643.
3. Convert `SiteTagPage` + swap its `.j-page` e2e assertions to `.j-scroll`.
4. Convert `UserTagPage` + drop orphaned imports + add a `UserTagPage` smoke
   e2e.
5. Final verification.

**Key risks / decisions:**

- **Coincidence moves in two files** (`render.rs` projector + `component.rs`
  CSR). Between Task 2 (projector flush) and Tasks 3/4 (CSR flush) the tag pages
  transiently diverge — harmless (intermediate, unmerged commits; CI gates only
  the PR tip), but the branch tip must have both flush. The `render.rs`
  coincidence test (Task 2) and wasm clippy + e2e (Tasks 3–5) are the guards.
- **The flatten must drop BOTH `j-page` and the inner posts-`<div>`** (spec D1)
  or the projector still diverges from `TimelineRows`
  (`j-scroll > div > article` vs `j-scroll > article`).

---

## Task 1: `TimelineRows` — optional `empty_text` prop

**Files:**

- Modify: `web/src/timeline/component.rs` (signature ~:106, empty render :117).

**Interfaces:**

- Produces: `TimelineRows` accepts
  `#[prop(default = "No posts yet.")] empty_text: &'static str`, rendered in the
  empty state as `<p>{empty_text}</p>`. Additive — existing call sites (home
  `:79`, cockpit `:103`, user timeline) omit it and keep "No posts yet.".

- [ ] **Step 1: Add the prop.** After the `tag_context` prop:

```rust
    #[prop(default = TagContext::SiteWide)]
    tag_context: TagContext,
    /// Empty-state message when there are no rows. Defaults to the generic
    /// "No posts yet."; the tag pages pass "No posts with this tag yet.".
    #[prop(default = "No posts yet.")]
    empty_text: &'static str,
) -> impl IntoView {
```

- [ ] **Step 2: Use it in the empty branch** (replace the hardcoded literal at
      :117):

```rust
                if rows.is_empty() {
                    view! { <p>{empty_text}</p> }.into_any()
                } else {
```

- [ ] **Step 3: Gate.** `cargo clippy -p web --target wasm32 -- -D warnings`,
      then `cargo xtask check --no-test`. Expected clean;
      home/cockpit/user-timeline call sites unchanged
      (`rg -n "TimelineRows" web/src/home/component.rs web/src/cockpit/component.rs`).

- [ ] **Step 4: Commit.**

```bash
git add web/src/timeline/component.rs
git commit -m "feat(web/timeline): TimelineRows takes an optional empty_text"
```

---

## Task 2: Flatten `render_timeline_page` to wrapper-free `j-scroll` (test-first)

Host-compiled + coverage-measured, so real red→green. Fixes #643's user-timeline
divergence (Profile is painted by this fn).

**Files:**

- Modify: `web/src/posts/render.rs` — the `body_covers_tag_page_headings`
  assertion (~:435) and `render_timeline_page` (:230-247).

**Interfaces:** none external (pure projector twin).

- [ ] **Step 1: Update the coincidence test to the flush structure (RED).** In
      `body_covers_tag_page_headings` (~:435), replace the `j-page` assertion
      with the same shape home's test pins (`:466`):

```rust
        assert!(
            site.contains("<div class=\"j-scroll\"><article class=\"j-post\">"),
            "{site}"
        );
```

- [ ] **Step 2: Run the test, verify it FAILS.** Run:
      `cargo nextest run -p web --lib posts::render::tests::body_covers_tag_page_headings`
      Expected: FAIL — current output is
      `<div class="j-scroll"><div class="j-page"><div>…`.

- [ ] **Step 3: Flatten `render_timeline_page`** (:230-247) — drop the `j-page`
      wrapper AND the inner posts-`<div>`, matching `render_body`'s
      `SiteTimeline` branch:

```rust
    let inner = if posts.is_empty() {
        format!("<p>{}</p>", escape_html(empty_text))
    } else {
        format!("{}{}", render_posts(posts, tag_ctx), render_load_more(has_more))
    };
    format!("{topbar}<div class=\"j-scroll\">{inner}</div>")
```

- [ ] **Step 4: Run the render tests, verify PASS.** Run:
      `cargo nextest run -p web --lib posts::render` Expected: PASS —
      `body_covers_tag_page_headings` green;
      `timeline_page_empty_states_differ_by_route` (asserts
      `<p>No posts with this tag yet.</p>` / `<p>No posts yet.</p>`) stays green
      (empty text unchanged); permalink/home tests unaffected.

- [ ] **Step 5: Gate + commit.** `cargo xtask check --no-test` (host
      clippy/fmt), then:

```bash
git add web/src/posts/render.rs
git commit -m "refactor(web/render): flush render_timeline_page to wrapper-free j-scroll (matches TimelineRows; fixes #643 profile coincidence)"
```

---

## Task 3: Convert `SiteTagPage` + swap its `.j-page` e2e assertions

**Files:**

- Modify: `web/src/posts/component.rs` — `SiteTagPage` (~:1810-1969).
- Modify: `end2end/tests/posts.spec.ts` — `.j-page` → `.j-scroll` at :788, :852,
  :853.

**Interfaces:**

- Consumes (already imported from #643): `TimelineState`, `spawn_load_more`,
  `TimelineRows`, `TagContext`;
  `list_posts_by_tag(tag, Option<UtcInstant>, Option<PostId>, Option<PageSize>)`;
  the new `empty_text` prop (Task 1).

- [ ] **Step 1: Rewrite the body**, mirroring `UserTimelinePage` (the merged
      #643 template in the same file). Replace the six ad-hoc signals +
      `ServerAction` + twin `Effect`s + inline rows with:

```rust
    let state = TimelineState::default();
    // CSR loading gate (see UserTimelinePage): TimelineState has no "never-loaded"
    // state, and this page gets `tag` from the route immediately and isn't always
    // seeded, so gate the empty-state flash on an unseeded client-nav first load.
    let loaded = RwSignal::new(false);

    // Public projector seed (#178/#179): adopt the seeded posts for a matching tag.
    if let Some(PageSeed::SiteTag { tag: seed_tag, page }) =
        use_context::<Option<PageSeed>>().flatten()
    {
        if tag.get_untracked().as_ref() == Some(&seed_tag) {
            state.adopt(page);
            loaded.set(true);
        }
    }

    // Initial-page fetch resolves on the client; seed the shared timeline once it
    // arrives (load-more appends via spawn_load_more). Canonical CSR, mirroring home.
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
        let Some(tag_value) = tag.get_untracked() else { return };
        spawn_load_more(state, move |created_at, post_id, limit| {
            list_posts_by_tag(tag_value, created_at, post_id, limit)
        });
    });

    let read_error = Memo::new(move |_| state.status.get().into_failure());
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();
```

Keep the `params`/`tag` Memo, `mutate_version`/`on_mutate`, and the
`initial_page` `Resource` (unchanged). The view keeps `FeedDiscovery` +
`Topbar`, then:

```rust
        {move || {
            if let Some(err) = read_error.get() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            if !loaded.get() {
                return view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any();
            }
            view! {
                <TimelineRows
                    state=state
                    on_mutate=on_mutate
                    on_load_more=on_load_more
                    empty_text="No posts with this tag yet."
                />
            }
                .into_any()
        }}
```

(No `tag_context` prop → `SiteWide` default, preserving current behavior.)
Delete the
`read_error`/`read_initial_loaded`/`read_timeline`/`read_has_more`/`read_pending`
closures and the `<div class="j-scroll"><div class="j-page">` wrapper.

- [ ] **Step 2: Swap the SiteTagPage e2e locators** in `posts.spec.ts` (the CSR
      view no longer has `.j-page`): `.j-page` → `.j-scroll` at: - `:788`
      `expect(page.locator(".j-scroll")).toContainText("Chip Nav Post")` -
      `:852` `waitForSelector(page, ".j-scroll")` - `:853-855`
      `expect(page.locator(".j-scroll")).toContainText("No posts with this tag yet.")`
      (The `"No posts with this tag yet."` text is preserved via `empty_text`.)

- [ ] **Step 3: Drop the now-orphaned `ListPostsByTag` import + gate.**
      `ListPostsByTag`'s only two uses (`:1855`, `:1900`) are gone after the
      `SiteTagPage` conversion, so remove `ListPostsByTag` from the
      `use crate::posts::{…}` block (keep `ListUserPostsByTag` — UserTagPage
      still uses it until Task 4; and `UtcInstant` — still used by UserTagPage's
      `next_cursor` until Task 4). Then
      `cargo clippy -p web --target wasm32 -- -D warnings`;
      `cargo xtask check --no-test`. Confirm
      `rg -n "ServerAction::<ListPostsByTag>|initial_loaded|next_cursor_created_at" web/src/posts/component.rs`
      no longer matches inside `SiteTagPage`.

- [ ] **Step 4: Browser-verify.** `cargo xtask e2e-local posts` — the two
      site-tag tests ("tag chip on permalink navigates to site tag listing",
      "editing a post updates tag chips and tag listing pages") green with the
      swapped locators.

- [ ] **Step 5: Commit.**

```bash
git add web/src/posts/component.rs end2end/tests/posts.spec.ts
git commit -m "refactor(web/posts): converge SiteTagPage onto shared TimelineState"
```

---

## Task 4: Convert `UserTagPage` + drop orphaned imports + add a smoke e2e

**Files:**

- Modify: `web/src/posts/component.rs` — `UserTagPage` (~:1978-2173) + the
  `use crate::posts::{…}` block (:20-26).
- Modify: `end2end/tests/posts.spec.ts` — add a `UserTagPage` smoke test.

**Interfaces:**

- Consumes:
  `list_user_posts_by_tag(username, tag, Option<UtcInstant>, Option<PostId>, Option<PageSize>)`.

- [ ] **Step 1: Rewrite the body** exactly as Task 3, with the username+tag
      specifics: keep the `username`/`tag` Memos + `initial_page` `Resource`;
      seed from `PageSeed::UserTag { username: seed_user, tag: seed_tag, page }`
      guarded on `username` **and** `tag`; adapter closure captures both:

```rust
    let on_load_more = Callback::new(move |()| {
        let Some(username_value) = username.get_untracked() else { return };
        let Some(tag_value) = tag.get_untracked() else { return };
        spawn_load_more(state, move |created_at, post_id, limit| {
            list_user_posts_by_tag(username_value, tag_value, created_at, post_id, limit)
        });
    });
```

The `TimelineRows` call passes **both** the tag context and the empty text (its
rows link `ForUser` today):

```rust
                match username.get() {
                    Some(user) => view! {
                        <TimelineRows
                            state=state
                            on_mutate=on_mutate
                            on_load_more=on_load_more
                            tag_context=TagContext::ForUser(user)
                            empty_text="No posts with this tag yet."
                        />
                    }.into_any(),
                    None => ().into_any(),
                }
```

(Guard `username.get()` `Some`, as in `UserTimelinePage`; the `None`/invalid
case is the validation-error path via `read_error`.) Keep `FeedDiscovery` +
`Topbar` chrome.

- [ ] **Step 2: Drop the now-orphaned imports.** Remove `ListUserPostsByTag`
      from the `use crate::posts::{…}` block (`ListPostsByTag` was already
      dropped in Task 3), and remove `use common::time::UtcInstant;` (`:40`) —
      its only uses were the two deleted `next_cursor_created_at` signals, so it
      is now fully unused. Keep `list_posts_by_tag`/`list_user_posts_by_tag`,
      `utc_instant_from_local`, `PostId`, and `TimelinePostSummary` (all still
      used elsewhere). Verify:
      `rg -n "ListPostsByTag|ListUserPostsByTag|UtcInstant" web/src/posts/component.rs`
      → nothing (`utc_instant_from_local` is a different symbol and stays).

- [ ] **Step 3: Add a `UserTagPage` smoke e2e** (no test navigates
      `/~user/tags/:tag` today). After the existing tag tests in
      `posts.spec.ts`:

```ts
test("user tag page lists that user's tagged posts", async ({
  registeredPage: page,
}) => {
  const { permalink } = await createPostViaApi(page, {
    body: "# User Tag Post\n\ncontent",
    tags: ["utaga"],
  });
  const userPath = permalink.match(/^(\/~[^/]+)\//)![1];
  await goto(page, `${userPath}/tags/utaga`);
  await waitForSelector(page, ".j-post-body");
  await expect(page.locator(".j-scroll")).toContainText("User Tag Post");
});
```

- [ ] **Step 4: Gate.** `cargo clippy -p web --target wasm32 -- -D warnings`;
      `cargo xtask check --no-test`. Confirm no `next_cursor`/`ServerAction`
      remain in either tag page.

- [ ] **Step 5: Browser-verify.** `cargo xtask e2e-local posts` — all tag
      tests + the new user-tag smoke test green.

- [ ] **Step 6: Commit.**

```bash
git add web/src/posts/component.rs end2end/tests/posts.spec.ts
git commit -m "refactor(web/posts): converge UserTagPage onto shared TimelineState; drop orphaned ServerAction imports"
```

---

## Task 5: Final verification (AC10, AC11)

- [ ] **Step 1: Static + coverage gate.** `cargo xtask validate --no-e2e`
      (foreground, `timeout: 600000`). Expected green — the updated
      `render_timeline_page` test keeps coverage; no new `cov:ignore`; CRAP OK.

- [ ] **Step 2: Wasm clippy.**
      `cargo clippy -p web --target wasm32 -- -D warnings` — clean.

- [ ] **Step 3: Browser verification.** `cargo xtask e2e-local posts` — full
      posts spec green (site-tag tests with swapped locators, the new user-tag
      smoke test, unaffected create/edit/timeline tests). Manually confirm no
      first-paint gutter flash on `/tags/:tag`, `/~user/tags/:tag`, and `/~user`
      (the #643 fix) — the projector and CSR are now the same flush structure
      (racy to assert; observe under a throttled client-side nav).

- [ ] **Step 4: No commit** (verification only). Hand off to `jaunder-ship`.

---

## Self-review

- **Spec coverage:** D1/D10→T2; D2→T1; D3→T3+T4; D4→T3(SiteWide
  default)+T4(ForUser); D5→T3+T4 loaded gate; D6→T3+T4 seed adopt; D7→T3+T4
  spawn_load_more; D8→T3+T4 read_error Memo; D9→T4 imports. AC1→T3/T4; AC2→T2;
  AC3→T1+T2; AC4→T1; AC5→T3/T4; AC6→T3/T4; AC7→T3/T4; AC8→T4; AC9→T2 (projector
  flush = profile coincidence); AC10→T5 + the e2e swaps/smoke in T3/T4; AC11→T5.
- **Placeholders:** none — T1/T2 carry exact Rust + the render-test red/green;
  T3/T4 carry the converted structure + adapter closures + the exact
  `TimelineRows` props and e2e locator swaps.
- **Type consistency:**
  `list_posts_by_tag(tag,…)`/`list_user_posts_by_tag(username,tag,…)` match
  `spawn_load_more`'s
  `FnOnce(Option<UtcInstant>,Option<PostId>,Option<PageSize>)`;
  `empty_text: &'static str` (T1) matches the `"…"` literals passed in T3/T4;
  `TagContext::ForUser(Username)` matches the widened `TimelineRows` prop from
  #643.

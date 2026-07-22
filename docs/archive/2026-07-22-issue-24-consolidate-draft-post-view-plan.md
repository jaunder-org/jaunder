# Consolidate DraftPreviewPage and PostPage — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Make `PostPage` (the permalink route) the single canonical post view —
teach `PostCard`'s action column about draft state (fixing #23), then delete
`DraftPreviewPage`, its route, and every `/draft/:id/preview` URL.

**Architecture:** The posts vertical is already on the ADR-0070 four-file layout
(`mod.rs`/`api.rs`/`server.rs`/`component.rs`, #323). All UI is in the wasm-only
`web/src/posts/component.rs`. `PostPage` already renders the author's own drafts
via `get_post`'s `find_draft_by_permalink_for_user` fallback, so consolidation
is mostly deletion plus one client-side reactive branch in `PostCard`'s action
column.

**Tech Stack:** Rust, Leptos CSR, `cargo`/`nextest`, `cargo xtask` gate,
Playwright (`end2end/`).

**Spec:**
`docs/superpowers/specs/2026-07-22-issue-24-consolidate-draft-post-view.md` —
this plan is "how"; consult the spec (Decisions 1–7, AC1–AC9) for "what/why".

## Global Constraints

- **Subsumes #23** — the draft-aware action column is the complete fix; #24's PR
  closes both.
- **`get_post_preview` is retained** — `EditPostPage` (`component.rs:1667`)
  still loads by `post_id`; only its `DraftPreviewPage` caller is removed.
- **No new `cov:ignore` / `crap:allow` markers** introduced by this change
  (AC9).
- **Gate:** the pre-commit hook runs full `cargo xtask check`; run it before
  each commit so it passes clean (**jaunder-commit**). **No `Co-Authored-By`
  trailer.**
- **Local e2e is reaped in this environment** — gate locally with
  `cargo xtask check` / `cargo xtask validate --no-e2e`; the Playwright suite
  (Task 6) is verified by CI's `{backend}×{browser}` matrix on the PR, not
  locally.
- `component.rs` is wasm-only (`#[cfg(target_arch = "wasm32")]`), so its changes
  are **not host-coverage-measured**; behavioral coverage of the UI is the Task
  6 e2e.

---

## Task list (one line each)

1. Delete `DraftPreviewPage`, its route, and the now-dead `render_delete_form`.
2. Thread `is_draft: bool` through `TimelinePostSummary` (+ round-trip test).
3. Make `PostCard`'s action column draft-aware: Publish (→ navigate to new
   permalink) vs Unpublish.
4. Replace `preview_url` with `permalink` across the result DTOs, builders,
   flash links, and the drafts row.
5. Confirm/extend `get_post`'s draft-visibility integration test (author /
   stranger / anon) — AC8.
6. Rewire the Playwright specs off the preview URL onto the permalink.

**Key risks/decisions:** deleting `DraftPreviewPage` orphans
`render_delete_form` (dead-code gate failure if not removed);
`CreatePostResult`/`UpdatePostResult` already have `permalink` but gated to
published posts — its population must change; publishing can move a draft's
permalink, so Publish must navigate to the _server-returned_ permalink; e2e
greenness is only provable on CI.

---

### Task 1: Delete `DraftPreviewPage`, its route, and `render_delete_form`

**Files:**

- Modify: `web/src/posts/component.rs` — delete `DraftPreviewPage` (`1512-1613`)
  and `render_delete_form` (`2069`+, its only caller was
  `DraftPreviewPage:1578`).
- Modify: `web/src/posts/mod.rs:70` — drop `DraftPreviewPage` from the
  `pub use component::{…}` re-export (keep the `get_post_preview` re-export at
  `:38`).
- Modify: `web/src/pages/mod.rs` — drop `DraftPreviewPage` from the import
  (`:34`) and delete the route (`149-156`, the `("draft", :post_id, "preview")`
  segment).

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces: `get_post_preview` remains a live `#[server]` fn (sole caller now
  `EditPostPage:1667`); no `/draft/:post_id/preview` route remains.

- [x] **Step 1: Delete the component, its route, and the orphaned helper.**
      Remove the four spans above. Leave `get_post_preview` (`api.rs:306`), its
      import in `component.rs:21` and `mod.rs:38`, and `EditPostPage`'s use of
      it (`component.rs:1667`) untouched.

- [x] **Step 2: Run the gate, verify it compiles clean.**

Run: `cargo xtask check --no-test` Expected: PASS — no `dead_code` warning for
`render_delete_form`, no unresolved `DraftPreviewPage` reference. (If clippy
flags any other now-unused import/helper that was exclusive to
`DraftPreviewPage`, remove it too.)

- [x] **Step 3: Confirm the preview route is gone.**

Run: `rg -n 'draft/.*preview|DraftPreviewPage' web/src` Expected: only the
`get_post_preview` server-fn name remains (no route, no component, no
`/draft/{…}/preview` string in `component.rs`; the `format!("/draft/…/preview")`
builders in `api.rs` are removed in Task 4).

- [x] **Step 4: Commit.**

```bash
git add web/src/posts/component.rs web/src/posts/mod.rs web/src/pages/mod.rs
git commit -m "refactor(web/posts): remove DraftPreviewPage and its preview route"
```

Run `cargo xtask check` first so the pre-commit gate passes clean
(**jaunder-commit**).

---

### Task 2: Thread `is_draft` through `TimelinePostSummary`

**Files:**

- Modify: `web/src/posts/api/listing.rs:37-52` — add `pub is_draft: bool,` to
  the struct.
- Modify: `web/src/posts/api.rs` round-trip test
  `timeline_summary_round_trips_rendered_html_via_trusted_rebuild` (`644-656`).
- Modify construction sites: `web/src/posts/server.rs:27-39`
  (`timeline_post_summary`); `web/src/posts/component.rs:1211-1225`
  (`PostPage`); `web/src/posts/render.rs:284-298` (`sample_summary`).

**Interfaces:**

- Consumes: nothing.
- Produces: `TimelinePostSummary { …, pub is_draft: bool }`. `PostCard` (Task 3)
  reads `post.is_draft`. Value is `fetched.is_draft` at `PostPage`, `false`
  everywhere a published-only row is built.

- [x] **Step 1: Add the field and pin it in the round-trip test.** Add
      `pub is_draft: bool,` after `pub is_author: bool,` in the struct. In the
      round-trip test, set `is_draft: true` in the constructed value and assert
      the round-tripped value's `is_draft == true` (the test already serializes
      → deserializes via the trusted-rebuild path; `is_draft` is a plain `bool`,
      no custom `serde`).

- [x] **Step 2: Run the test, verify it fails.**

Run: `cargo nextest run -p web timeline_summary_round_trips` Expected: FAIL to
**compile** — the four other `TimelinePostSummary` literals (`server.rs`,
`component.rs` PostPage, `render.rs` sample) are missing the new field.

- [x] **Step 3: Populate every construction site.**
  - `server.rs:27-39` `timeline_post_summary` → `is_draft: false` (it
    early-returns `None` unless `published_at.is_some()`, so it only ever builds
    a published row).
  - `component.rs:1211-1225` `PostPage` → `is_draft: fetched.is_draft`.
  - `render.rs:284-298` `sample_summary` → `is_draft: false`.

- [x] **Step 4: Run the test, verify it passes.**

Run: `cargo nextest run -p web timeline_summary_round_trips` Expected: PASS.

- [x] **Step 5: Commit.**

```bash
git add web/src/posts/api/listing.rs web/src/posts/api.rs web/src/posts/server.rs web/src/posts/component.rs web/src/posts/render.rs
git commit -m "feat(web/posts): carry is_draft on TimelinePostSummary"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 3: Make `PostCard`'s action column draft-aware

**Files:**

- Modify: `web/src/posts/component.rs` — `PostCard` (`185-268`).

**Interfaces:**

- Consumes: `TimelinePostSummary.is_draft` (Task 2); `PublishPost` action struct
  and `PublishPostResult { …, permalink: String }` (`api.rs:101-108`, already
  imported for `render_draft_row` — confirm `PublishPostResult` is in scope, add
  to the `use` at `component.rs:21` if not).
- Produces: `PostCard` renders `[Edit, Publish, Delete]` for a draft (Publish
  navigates to the returned permalink) and `[Edit, Unpublish, Delete]` for a
  published post.

_No host-level unit test exists for wasm component behavior; AC2/AC4 are covered
by the Task 6 e2e. The verification here is the gate (compile + wasm clippy).
The body is written out because no unit test can pin it._

- [x] **Step 1: Add draft state, a publish action, and a publish-navigate
      effect.** Near the existing `let post_id = post.post_id;` (line 201) add
      `let is_draft = post.is_draft;`. Alongside the existing `unpublish_action`
      (line 204) add:

```rust
let publish_action = ServerAction::<PublishPost>::new();
```

After the existing unpublish effect (lines 215-222) add:

```rust
Effect::new_isomorphic(move |_| {
    if let Some(Ok(published)) = publish_action.value().get() {
        // Publishing can move the permalink (created_at -> published_at date),
        // so navigate to the server-returned canonical permalink, not the
        // (possibly now-stale) current URL. Mirrors the deleted DraftPreviewPage.
        if let Some(window) = web_sys::window() {
            let _ = window.location().replace(&published.permalink);
        }
    }
});
```

- [x] **Step 2: Branch the primary action button on `is_draft`.** Replace the
      fixed Unpublish `<button>` (lines 233-241) with a `primary_action` chosen
      by `is_draft`, built before `action_col` and moved into it:

```rust
let primary_action = if is_draft {
    view! {
        <button
            type="button"
            class="j-btn"
            on:click=move |_| {
                let confirmed = web_sys::window()
                    .and_then(|w| { w.confirm_with_message("Publish this draft?").ok() })
                    .unwrap_or(false);
                if confirmed {
                    publish_action.dispatch(PublishPost { post_id });
                }
            }
        >
            "Publish"
        </button>
    }
    .into_any()
} else {
    view! {
        <button
            type="button"
            class="j-btn"
            on:click=move |_| {
                unpublish_action.dispatch(UnpublishPost { post_id });
            }
        >
            "Unpublish"
        </button>
    }
    .into_any()
};
```

In `action_col` (lines 227-258) place `{primary_action}` between the Edit `<a>`
and the Delete `<button>` (Edit and Delete are unchanged in both arms).

- [x] **Step 3: Run the gate, verify it compiles clean (incl. wasm clippy).**

Run: `cargo xtask check --no-test` Expected: PASS — no unused-import/variable
warnings; `PublishPost`/`PublishPostResult` resolve.

- [x] **Step 4: Commit.**

```bash
git add web/src/posts/component.rs
git commit -m "fix(web/posts): draft-aware PostCard action column (closes #23)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 4: Replace `preview_url` with `permalink` across DTOs, builders, and flashes

**Files:**

- Modify: `web/src/posts/api.rs` — `CreatePostResult` (`60-70`),
  `UpdatePostResult` (`72-81`), `DraftSummary` (`83-99`); builders `create_post`
  (`230-244`), `update_post` (`407-419`), `list_drafts` (`486-501`).
- Modify: `web/src/posts/component.rs` — `InlineComposer` (`764-767`),
  `CreatePostPage` flash (`1081-1096`), `EditPostPage` publish-redirect effect
  (`1646`) **and** flash (`1924-1925`), `render_draft_row` (`2039`).
- Modify: `web/src/posts/parse.rs:153` — the `DraftSummary` fixture.
- Modify: `server/tests/web/web_posts.rs` — every reader of the two DTOs'
  `permalink`, which becomes `String`:
  `create_post_persists_rendered_published_post` (`preview_url` `~136`, plus
  `created.permalink.is_some()` `:139` and `.as_deref()` `:171`);
  `create_post_accepts_slug_override_and_saves_draft` (`preview_url` `~321`,
  plus the **semantic flip** at `:324`); the `update_post` test's
  `updated.permalink.is_some()` (`:1091`); the delete-flow test's
  `created.permalink.unwrap()` (`:2474`). (The
  `published.permalink`/`post.permalink` hits at `:1675/:1989/:2107` are
  `PublishPostResult`/`TimelinePostSummary` — already `String`, untouched.)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces: `CreatePostResult`/`UpdatePostResult` carry `permalink: String`
  (always the post's canonical permalink, drafts included) and **no**
  `preview_url`. `DraftSummary` carries `permalink: String` and **no**
  `preview_url`. No code emits `/draft/:id/preview`.

- [x] **Step 1: Update the integration assertions (the contract).** In
      `server/tests/web/web_posts.rs`, adapt every reader of the two DTOs'
      now-`String` `permalink` and drop the `preview_url` assertions:
  - `create_post_persists_rendered_published_post`: remove the
    `created.preview_url` assertion (`~136`); change
    `assert!(created.permalink.is_some())` (`:139`) to a non-empty
    published-permalink assertion (e.g.
    `assert!(created.permalink.contains("/~"))`); change
    `created.permalink.as_deref()` (`:171`) to `created.permalink.as_str()` (or
    `&created.permalink`), preserving the existing comparison.
  - `create_post_accepts_slug_override_and_saves_draft`: remove the
    `created.preview_url` assertion (`~321`); **flip the semantics** at `:324` —
    replace `assert!(created.permalink.is_none())` with an assertion that the
    draft now carries its canonical permalink:
    `assert!(!created.permalink.is_empty())` and (stronger) that it equals the
    `created_at`-based `PostRecord::permalink()` form (the same value
    `DraftSummary.permalink` produces). This encodes Decision 6.
  - `update_post` test: change `assert!(updated.permalink.is_some())` (`:1091`)
    to a non-empty assertion.
  - delete-flow test: change `let permalink = created.permalink.unwrap();`
    (`:2474`) to `let permalink = created.permalink;`.

- [x] **Step 2: Run those tests, verify they fail.**

Run: `cargo nextest run -p jaunder --test integration create_post` Expected:
FAIL to compile — `preview_url` no longer referenced but the field still exists
/ `permalink` type still `Option`. (Proves the assertions bind to the new
field.)

- [x] **Step 3: Change the DTOs and builders.**
  - `CreatePostResult`: remove `pub preview_url: String,`; change
    `pub permalink: Option<String>,` → `pub permalink: String,`.
  - `UpdatePostResult`: same two edits.
  - `DraftSummary`: remove `pub preview_url: String,` (leave
    `pub permalink: String,`).
  - `create_post` builder (`230-244`): delete the
    `preview_url = format!("/draft/…")` line; set
    `permalink: record.permalink()` (drop the `published_at.is_some().then(…)`
    gate — a draft's `permalink()` is its `created_at`-based canonical URL).
  - `update_post` builder (`407-419`): same — `permalink: record.permalink()`,
    remove `preview_url`.
  - `list_drafts` builder (`486-501`): remove the `preview_url` line (keep
    `permalink`).

- [x] **Step 4: Update the flash/link consumers.**
  - `InlineComposer` (`764-767`): `let url = created.permalink.clone();` (drop
    the `.unwrap_or_else(|| created.preview_url…)` fallback).
  - `CreatePostPage` (`1081-1096`): collapse the two links (`preview-link` +
    published-only `permalink-link`) into a single
    `<a data-test="permalink-link" href=created.permalink.clone()>"View post"</a>`.
  - `EditPostPage` publish-redirect effect (`1646`): `updated.permalink` is now
    `String`, so `if let Some(ref permalink) = updated.permalink { … }` no
    longer compiles — replace with
    `window.location().replace(&updated.permalink)` directly (the enclosing
    `updated.published_at.is_some()` guard stays, so navigation remains
    publish-only).
  - `EditPostPage` flash (`1924-1925`):
    `<a data-test="permalink-link" href=updated.permalink.clone()>"View post"</a>`.
  - `render_draft_row` (`2039`): delete the
    `<a href=draft.preview_url>"Preview"</a>` line and its trailing separator,
    leaving only the `"Permalink"` link.
  - `parse.rs:153`: remove `preview_url: …` from the `DraftSummary` fixture.

- [x] **Step 5: Run the tests and gate, verify green.** Run the full `web_posts`
      suite (the changed assertions span `create_post`, `update_post`, and the
      delete-flow tests):

Run: `cargo nextest run -p jaunder --test integration web_posts` Expected: PASS.
Run: `cargo xtask check` — Expected: PASS (fmt/clippy/coverage + full
instrumented tests; no new markers). A missed `permalink` type site (if any)
surfaces here as a compile error.

- [x] **Step 6: Confirm no preview URL is emitted anywhere.**

Run: `rg -n 'preview_url|/draft/.*preview' web/src server/tests` Expected: no
hits (AC6).

- [x] **Step 7: Commit.**

```bash
git add web/src/posts/api.rs web/src/posts/component.rs web/src/posts/parse.rs server/tests/web/web_posts.rs
git commit -m "refactor(web/posts): link drafts by canonical permalink, drop preview_url"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 5: Confirm/extend `get_post` draft-visibility test (AC8)

**Files:**

- Modify: `server/tests/web/web_posts.rs` —
  `get_post_returns_draft_to_author_only` (`~558`).

**Interfaces:**

- Consumes: `get_post` (unchanged). Produces: a test asserting the author sees
  the draft at its permalink, and a different signed-in user and an anonymous
  visitor are denied.

- [x] **Step 1: Read the test and identify missing cases.** AC8 requires all
      three: author sees; a _different_ authenticated user is denied; an
      anonymous request is denied. If any case is absent, add it.

- [x] **Step 2: Add the missing assertion(s).** (No change needed — all three
      cases already present: anon→NOT_FOUND, stranger→NOT_FOUND, author→OK with
      `is_draft:true`, in `get_post_returns_draft_to_author_only`; reinforced by
      `get_post_hides_drafts_from_guests`.) For the denial cases, call
      `get_post` with the draft's permalink params as (a) a second authenticated
      user and (b) no auth, asserting each returns not-found / is denied the
      draft (match the crate's existing denial assertion style —
      `WebError::not_found` or the test's helper). If all three cases already
      exist, record that in the commit body and skip to Step 4.

- [x] **Step 3: Run the test, verify it passes.** (Passed under Task 4's full
      `cargo xtask check` instrumented run — both backends, `tests-ok`.)

Run:
`cargo nextest run -p jaunder --test integration get_post_returns_draft_to_author_only`
Expected: PASS.

- [x] **Step 4: Commit.** (No commit — AC8 was already covered, so this task
      made no code change.)

```bash
git add server/tests/web/web_posts.rs
git commit -m "test(web/posts): draft permalink hidden from strangers and anon"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 6: Rewire the Playwright specs onto the permalink

**Files:**

- Modify: `end2end/tests/posts.spec.ts`.

**Interfaces:**

- Consumes: the new single-canonical behavior. Produces: e2e coverage for AC1,
  AC2, AC4, AC5, AC6.

_Local heavy e2e is reaped here; do not run it locally — it is verified by CI's
`{backend}×{browser}` matrix on the PR (Task-level "expected PASS" refers to
CI)._

- [x] **Step 1: Rewire the preview-anchored assertions to the permalink.** In
      each of the tests below (line refs from current HEAD), replace
      `[data-test="preview-link"]` / `/\/draft\/(\d+)\/preview/` extraction and
      the "Preview draft" flash-label assertion with the `permalink-link` /
      permalink-URL equivalent:
  - `L570` "inline composer: draft flash is a link to the draft preview URL" —
    rename to "…is a link to the draft permalink" and assert the flash's
    `permalink-link` href is the draft's permalink (not `/draft/…/preview`).
  - `L81-94`, `L127-145`, `L160-189` — replace preview-link
    extraction/visibility with the `permalink-link` equivalent; drop the
    "Preview draft" label assertion.
  - `L672-686` — if this test reaches the edit page via a preview link, navigate
    via the permalink/edit-url instead (the edit page still loads chips via the
    retained `get_post_preview`, so the chip-loading assertion is unchanged).

- [x] **Step 2: Update the draft-lifecycle test to assert publish-navigation.**
      In "draft lifecycle: create, view, edit, and publish" (`L226-265`),
      navigate to the draft via its permalink, click **Publish**, confirm, and
      assert the browser lands on the returned **published** permalink and
      renders the now-published post (AC4). Assert the drafts listing links only
      the permalink — no "Preview" link / no `/draft/…/preview` href (AC5).

- [x] **Step 3: Assert no preview URL survives (AC6).** Ensure no spec still
      references `/draft/…/preview` or `preview-link`.

Run: `rg -n 'preview-link|/draft/.*preview' end2end/tests` Expected: no hits.

- [x] **Step 4: Commit.**

```bash
git add end2end/tests/posts.spec.ts
git commit -m "test(e2e): drive drafts via canonical permalink, not preview URL"
```

Run `cargo xtask check` first (**jaunder-commit**). Final branch validation is
the CI e2e matrix on the PR (**jaunder-ship**).

---

## Self-review

- **Spec coverage:** AC1 (T1 route delete), AC2/AC3 (T3 draft/published
  columns), AC4 (T3 publish-navigate + T6 e2e), AC5 (T4 draft row + T6), AC6
  (T4 + T6 `rg` gates), AC7 (T2 round-trip), AC8 (T5), AC9 (gate run before
  every commit; component is wasm-only, no host markers). Decisions 1–7 all map
  to tasks; Decision 7 (retain `get_post_preview`) is a Global Constraint
  enforced by T1's scope.
- **No separable concerns** surfaced — the whole change is one coherent
  consolidation; no first-task issue filing needed.
- **Type consistency:** `permalink` is `String` on
  `CreatePostResult`/`UpdatePostResult` (T4), matching the existing
  `DraftSummary.permalink: String` and `PublishPostResult.permalink: String`;
  `is_draft: bool` on `TimelinePostSummary` (T2) is read by `PostCard` (T3) and
  set from `PostResponse.is_draft`.
- **Coverage note:** the coverage-measured (host) changes are the `api.rs`
  DTO/builder edits and the `TimelinePostSummary` field, exercised by the T2
  round-trip test and the T4 integration assertions; the untested-at-unit-level
  work is confined to wasm-only `component.rs` and covered by T6 e2e.

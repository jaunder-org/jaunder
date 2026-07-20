# Plan — #323: converge the `posts` vertical onto the file-level host/wasm split

**Spec:** `docs/superpowers/specs/2026-07-20-issue-323-posts-vertical.md`
(what/why). **ADRs:** 0070 (layout), 0072 (`UtcInstant` boundary), 0055 (extract
pure host-tested). **Templates:** shipped `web/src/media/` (api split +
wasm-only `component.rs`), `web/src/auth/`. **For agentic workers:** drive with
**`jaunder-iterate`**, delegating a task to a subagent via
**`jaunder-dispatch`** when useful (several tasks here are large, mechanical
moves — good delegation candidates); tick checkboxes in real time. Gate each
commit with **`jaunder-commit`**.

## Review header

**Goal.** Converge the largest web vertical into the ADR-0070 shape —
`posts/{mod.rs` wiring / `api.rs` (15 `#[server]` fns + DTOs) / `server.rs`
host-only / `render.rs` (pure render twins, host-compiled + coincidence-tested)
/ `component.rs` wasm-only UI`}` — pulling the posts widgets out of shared
`pages/ui.rs`, carving the render twins out of shared `render/`, folding #299,
migrating the datetime helper to a host-tested `chrono` conversion, and retiring
all `cov:ignore` / `read_signal!`.

**Scope.**

- _In:_ Tasks 1–7 below. #91 already typed the timestamps (`UtcInstant`) — not
  re-done.
- _Out:_ #312 (shared-UI remainder: `escape_html`, `Topbar`, `PageSeed`), #330
  (Router), #327/#328 (final relocation of `SubscribeButton`/tag-pages — they
  land in posts here), #516 (nav-primitive swap for `location().replace`).

**Tasks.**

- [ ] 1. `api.rs` split — 15 `#[server]` fns + DTOs out of
     `mod.rs`+`listing.rs`; `mod.rs` → wiring; `server.rs` absorbs listing's
     server helpers; registrar paths stable.
- [ ] 2. `render.rs` carve — pure render twins out of `render/mod.rs` +
     coincidence tests; `escape_html`/`PageSeed` stay shared.
- [ ] 3. Widget pull — posts `#[component]`s + helpers out of `pages/ui.rs` into
     `component.rs`; re-export; repoint
     `timeline.rs`/`cockpit.rs`/`pages/ui.rs`.
- [ ] 4. Routed components — `pages/posts.rs` → `component.rs`; retire
     `cov:ignore`; extract pure `post_data`/`render_draft_row` helpers
     host-tested; inline `read_signal!`; carry `location().replace`; repoint
     `pages/mod.rs`; delete `pages/posts.rs`.
- [ ] 5. datetime → `chrono` — host-tested local→UTC `UtcInstant` constructor in
     `common::time`; drop `js_sys::Date`.
- [ ] 6. #299 — fold `create_post`/`update_post` into typed arg-structs; drop
     the `too_many_arguments` allow; e2e-verify.
- [ ] 7. Full `cargo xtask validate` (e2e + coincidence tests).

**Key risks / decisions.**

- **Render-twin carve is the delicate part** (Task 2): the twins are shared by
  the server projector AND the reactive `PostDisplay`, so `render.rs` is
  **host-compiled** (ungated), and the **coincidence tests** guard flash-free
  byte-equality. `PageSeed` stays in `render/` (shared seam).
- **Shared-widget coupling** (Task 3): `PostCard`→`timeline.rs`,
  `PostCreateForm`/ `InlineComposer`→`cockpit.rs`. Re-export from `posts/mod.rs`
  and repoint, or those break.
- **Registrar stability** (Task 1): 15 `#[server]` arg-structs registered by
  path in `server/tests/helpers/mod.rs` — the `api.rs` split must re-export them
  unchanged (`server-fn-registrar` gate). Verify before the UI moves.
- **datetime chrono siting** (Task 5): put the conversion in `common::time`
  (beside `UtcInstant`, already wasm-available) rather than un-gating `web`'s
  server-only chrono — respects #91/ADR-0072 (no chrono in `web` wasm build,
  boundary stays `UtcInstant`).
- **wasm-clippy is load-bearing** for the now-wasm-only UI.
- **Ordering:** api → render → widgets → components, because components depend
  on the widgets which depend on the render twins.

## Global constraints

- Rust, `cargo`. No `Co-Authored-By`. ADR-0070: `component.rs` wasm-only by its
  `mod` line, zero cfgs inside, no `cov:ignore`/`#[component]`-exemption for UI;
  `mod.rs` wiring only; `target_arch` only on `mod`/`pub use`. ADR-0055: pure
  logic → ungated host-tested files before gating; no fake host stub. ADR-0072:
  no `chrono` type in any `#[server]` signature.
- After any web threading change, gate with `cargo xtask check` (coverage build,
  `--all-features`) — not a bare default `cargo check` (memory:
  default-check-skips-server-gated-web). Pre-commit hook runs the full gate; run
  it green first. Serialize edit→gate→commit.
- Wasm-only helper consumed only by `component.rs` + tests must be `pub` +
  re-exported or it's host `dead_code` (memory:
  wasm-only-pure-helper-must-be-exported).
- **Delegation briefs** (jaunder-dispatch) must restate house rules: scope
  searches to the worktree (no FS-wide/`~/.cargo`), no `pkill`, no
  `#[allow]`/`#[expect]` without approval, don't commit.

---

## Task 1 — `api.rs` split (mod.rs + listing.rs → api.rs; mod.rs wiring)

**Files.**

- New `web/src/posts/api.rs`: move the wire DTOs (`CreatePostResult`,
  `UpdatePostResult`, `AudienceSelection`, `DraftSummary`, `PublishPostResult`,
  `PostResponse` from `mod.rs`; `TimelinePostSummary`, `TimelinePage` from
  `listing.rs`), `deserialize_rendered_html`, all **15 `#[server]` fns** (10
  from `mod.rs`, 5 from `listing.rs`), and the grouped
  `#[cfg(feature = "server")]` support block. To keep it readable, `api.rs` MAY
  carry an internal `#[path]`-free `mod listing;` submodule for the 5 timeline
  fns + their 2 DTOs, re-exported (`pub use listing::*;`) — decide by file size
  while implementing.
- `web/src/posts/mod.rs` → **wiring only** (mirror `media/mod.rs`): `//!` doc,
  `mod api;`, `#[cfg(feature="server")] mod server;`, `mod render;` (Task 2),
  `#[cfg(target_arch="wasm32")] mod component;` (Tasks 3–4), and `pub use`
  re-exports preserving every `crate::posts::…` path (DTOs, fns, generated
  arg-structs, `deserialize_rendered_html` where crate-visible).
- `web/src/posts/server.rs`: absorb `listing.rs`'s server-only fetch helpers
  (`page_from_rows`, `fetch_user_posts`, `fetch_local_timeline`,
  `fetch_posts_by_tag`, `fetch_user_posts_by_tag`) if they don't need to stay
  beside their `#[server]` callers; otherwise keep them in the `api.rs`
  `listing` submodule under `#[cfg(feature="server")]`. Keep `server.rs`'s
  existing marshalling.

**Interfaces.** Every
`web::posts::{CreatePost, UpdatePost, GetPost, …, ListUserPosts, …}`
arg-struct + DTO + fn path resolves unchanged. No behavior change.

**Test / Run.** No new test (pure move). Prove path + registrar stability:

- `cargo xtask check` green (host + wasm).
- Registrar: `server/tests/helpers/mod.rs` still compiles;
  `cargo nextest run -p jaunder --test integration` builds. The
  `server-fn-registrar` xtask gate must stay green.

**Commit** (`jaunder-commit`):
`refactor(web): split posts api.rs from mod.rs/listing.rs (wiring-only mod.rs) (#323)`.

---

## Task 2 — `render.rs` carve (pure twins → posts/render.rs + coincidence tests)

**Files.**

- New `web/src/posts/render.rs` (ungated, host-compiled): move from
  `web/src/render/mod.rs` the twins `permalink_article`, `render_posts`,
  `PostView<'a>`, `render_post_article`, `render_post_inner`,
  `render_post_content`, `render_timeline_page`, `format_post_time`,
  `render_body`, **with their `#[cfg(test)]` coincidence tests**. Keep
  visibility so both the server projector and `PostDisplay` (Task 3, in
  `component.rs`) can call them (`pub(crate)` + `mod.rs` re-export where the
  projector reaches them cross-module).
- `web/src/render/mod.rs`: **keep** `escape_html` and `PageSeed` (shared — #312
  / the projector seam). Repoint its internal callers of the moved twins to
  `crate::posts::render::…`. Repoint the projector entry points that render
  permalink/timeline pages to the new home.
- `web/src/posts/mod.rs`: add `mod render;` + needed re-exports.

**Interfaces.** The projector's server-painted HTML and the reactive
`PostDisplay` still call the SAME pure fns → byte-coincidence preserved.
`PageSeed` path unchanged.

**Test / Run.** The moved **coincidence tests** are the safety net
(host-compiled):

- `cargo nextest run -p web` — the carved coincidence + render tests PASS.
- `cargo xtask check` green (host + wasm).

**Commit** (`jaunder-commit`):
`refactor(web): carve posts render twins into posts/render.rs (#323)`.

---

## Task 3 — Widget pull (posts widgets: pages/ui.rs → posts/component.rs)

**Files.**

- New `web/src/posts/component.rs` (wasm-only;
  `#[cfg(target_arch="wasm32")] mod component;` in `mod.rs`; **zero cfgs
  inside**): move the `#[component]`s `ComposerFields`, `PostCreateForm`,
  `InlineComposer`, `PostCard`, `PostDisplay`, `AudiencePicker`, `TagInput`, and
  the private helper `marker_matches` from `web/src/pages/ui.rs`. Repoint their
  imports (`crate::posts::render::{PostView, render_post_inner, …}`,
  `crate::posts::{…DTOs}`, shared leaves
  `crate::{topbar,avatar,icon,taglist}::…`). Leave
  `local_datetime_to_utc_rfc3339`/`publish_at_from_local` for Task 5.
- `web/src/posts/mod.rs`:
  `#[cfg(target_arch="wasm32")] pub use component::{PostCard, PostDisplay, PostCreateForm, InlineComposer, ComposerFields, AudiencePicker, TagInput};`
  (the ones consumed outside posts must be re-exported).
- Repoint external call sites: `web/src/pages/timeline.rs` (`PostCard`),
  `web/src/pages/cockpit.rs` (`PostCreateForm`/`InlineComposer`),
  `web/src/pages/mod.rs` (drop the `pub use ui::{…PostCard, PostDisplay…}` for
  the moved items), `web/src/pages/ui.rs` (remove the moved defs).
  `pages/posts.rs` still imports these — it now resolves them from
  `crate::posts::…` (transitional until Task 4).
- Confirm no dangling refs:
  `rg 'ui::(PostCard|PostDisplay|ComposerFields|PostCreateForm|InlineComposer|AudiencePicker|TagInput|marker_matches)'`.

**Test / Run.** Wasm-only UI (not host-tested); type-check via
`cargo xtask check` (wasm-clippy load-bearing). Behavioral proof is the Task 7
e2e.

**Commit** (`jaunder-commit`):
`refactor(web): pull posts widgets into posts/component.rs (#323)`.

---

## Task 4 — Routed components (pages/posts.rs → component.rs) + delete pages/posts.rs

**Files.**

- `web/src/posts/component.rs`: move the 9 routed `#[component]`s
  (`CreatePostPage`, `PostPage`, `UserTimelinePage`, `DraftPreviewPage`,
  `EditPostPage`, `DraftsPage`, `SiteTagPage`, `UserTagPage`) +
  `SubscribeButton` + the helper components. Repoint imports to
  `crate::posts::…`, `crate::media::MediaUpload`, `crate::subscriptions::…`,
  `crate::tags::TagSummary`. Carry the three `web_sys::…location().replace`
  sites as-is.
- **Retire `cov:ignore`** on `permalink_first_paint`, `render_draft_row`,
  `render_delete_form`, `render_delete_result` (wasm-only file → not
  coverage-measured; no markers needed).
- **Extract pure helpers, host-tested** (into an ungated posts file — `api.rs`
  or a small `posts/parse.rs`): the `post_data` permalink-param parsing (route
  params → typed username/year/month/day/slug) and `render_draft_row`'s
  title/schedule-badge logic. Add `#[cfg(test)] mod tests` (valid/invalid
  params; badge cases). These are `pub` + re-exported if only
  `component.rs`/tests consume them (dead_code rule).
- **Inline `read_signal!`** → `.get()` at all 17 posts sites (folds #304).
- `web/src/pages/mod.rs`: repoint the 8 route `use`s to `crate::posts::…`;
  `<Route>`s unchanged (App/Router relocation is #330). Delete `pub mod posts;`
  (pages).
- Delete `web/src/pages/posts.rs`.

**Test / Run.** `cargo nextest run -p web` (the new host-tested extractions
PASS); `cargo xtask check` green (host + wasm).

**Commit** (`jaunder-commit`):
`refactor(web): relocate posts pages into posts/component.rs; retire cov:ignore; inline read_signal! (#323)`.

---

## Task 5 — datetime helper → `chrono` (host-tested)

**Files.**

- `common/src/time.rs`: add a pure `pub fn` beside `UtcInstant` — e.g.
  `utc_instant_from_local(local: &str) -> Option<UtcInstant>` — parsing the
  `datetime-local` wall-clock (`YYYY-MM-DDTHH:MM`) via `chrono::NaiveDateTime` +
  `chrono::Local.from_local_datetime(...).single()` → `UtcInstant`. Ensure
  `common`'s `chrono` has `clock` (+ `wasmbind`, which chrono enables on wasm) —
  add the feature(s) if absent. `#[cfg(test)] mod tests`: fixed-TZ cases (set
  `TZ`), incl. empty/invalid → None.
- `web/src/posts/component.rs`: replace `publish_at_from_local`'s body to call
  `common::time::utc_instant_from_local`; delete the `js_sys::Date`-based
  `local_datetime_to_utc_rfc3339`. No `js_sys::Date` remains in the datetime
  path.

**Test / Run.** `cargo nextest run -p common utc_instant_from_local` — PASS;
`cargo xtask check` green (host + wasm — confirm `common`'s chrono
clock/wasmbind compiles for wasm).

**Commit** (`jaunder-commit`):
`refactor(common): host-tested chrono local→UtcInstant; drop js_sys::Date (#323)`.

---

## Task 6 — #299: typed `create_post`/`update_post` arg-structs

**Files.**

- `web/src/posts/api.rs`: define
  `pub struct CreatePostArgs { body: PostBody, format: String, slug_override: Option<Slug>, publish: bool, publish_at: Option<UtcInstant>, tags: Option<Vec<TagLabel>>, summary: Option<String>, audience: Option<AudienceSelection> }`
  (`#[derive(Serialize, Deserialize, …)]`) and `UpdatePostArgs` (same +
  `post_id: PostId`). Change `create_post`/`update_post` to take a single
  `args: <T>Args` (keep `input = Json`), destructure in the body. **Remove
  `#[allow(clippy::too_many_arguments)]`** from both.
- Repoint the caller components (`CreatePostPage`/`EditPostPage` in
  `component.rs`) to build the arg-struct; update the generated
  `ServerAction`/`ActionForm` usage accordingly. Register the (unchanged-named)
  `CreatePost`/`UpdatePost` structs — paths already stable.

**Interfaces.** Wire nests under the arg name (leptos's own client, internal).
**Verify via the create/edit-post e2e** (Task 7) that create/edit/schedule still
round-trip.

**Test / Run.** `cargo xtask check` green; targeted
`cargo nextest run -p jaunder` for any create/update server-side test.

**Commit** (`jaunder-commit`):
`refactor(web): fold create_post/update_post into typed arg structs (#299) (#323)`.

---

## Task 7 — Full validate

**Gate.** `cargo xtask validate` (static + wasm-clippy + coverage + full e2e
matrix), foreground/background per length. Load-bearing: the posts e2e
(`posts.spec.ts` + `visibility`/`feeds`/`atompub`/`media`/`authed-flash` posts
flows) and the **render-twin coincidence tests**. Confirm create/edit/schedule
(#299 wire) and the datetime control (chrono) across all
`{sqlite,postgres}×{chromium,firefox}` combos.

**No ADR** — #323 applies ADR-0070/0072/0055; it records no new cross-cutting
decision.

**Done when:** spec acceptance-floor items 1–9 hold and `cargo xtask validate`
is green.

## Self-review

- Each task ends green-gated + committed; ordering (api → render → widgets →
  components → datetime → #299) respects the dependency chain (components need
  widgets need render twins).
- Registrar/path stability verified in Task 1 before any UI move; coincidence
  tests move with the twins in Task 2 (never a window where the projector
  coincidence is unguarded).
- No `mod.rs` items, no in-`component.rs` cfg, no host stub, no UI `cov:ignore`,
  no chrono in a `#[server]` sig — enforced by the gate + spec floor.
- Large mechanical moves (Tasks 1, 3, 4) are delegation candidates
  (`jaunder-dispatch`); the render carve (2) and #299 (6) are behavioral — keep
  close.

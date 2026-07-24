# Spec — #328 web(tags): converge the tags vertical onto the co-located Leptos layout

- Issue: [#328](https://github.com/jaunder-org/jaunder/issues/328)
- Milestone: Web: canonical Leptos CSR convergence (#11)
- Date: 2026-07-24
- Governing ADR:
  [ADR-0070](../../adr/0070-web-vertical-wasm-only-component-files.md)
  (file-level host/wasm split; wasm-only `component.rs`), incl. its #527
  amendment (shared leaf widgets are top-level modules) and #530 amendment
  (`mod.rs` wiring only).

## Problem

The "tags" surface is scattered, and its one piece of genuinely tags-domain
**UI** lives in the wrong vertical:

- `tags/` (`mod.rs` + `api.rs`) holds the `list_tags` autocomplete `#[server]`
  endpoint (+ `TagSummary` wire type, `DEFAULT_TAG_LIMIT`/`MAX_TAG_LIMIT`). This
  is already ADR-0070-shaped for an endpoint-only vertical.
- **`TagInput`** — the tags autocomplete widget and the _only_ in-crate consumer
  of `crate::tags::list_tags` — sits inside the ~2300-line `posts/component.rs`
  (`posts/component.rs:828`), a leftover of the #323 posts convergence. So the
  tags vertical has a server fn but no UI home; the acceptance-floor requirement
  "tags UI
  - server fns + wire types co-located in one feature module" is unmet.

Separately, an **opportunistic tidy carried on this issue (from #505)**: the two
tag-timeline pages still wrap their reactive `Topbar` `title`/`sub` props in
`Signal::derive(move || …)`. Since `Topbar`'s props are `leptos::TextProp`, a
bare `move || …` closure converts directly; the wrapper is redundant (neither
form memoizes).

### Correction to the issue text

The issue says the tag-timeline pages live in `web/src/pages/posts.rs` and
should "relocate into this vertical." That text is **stale**: there is no
`pages/posts.rs`; `SiteTagPage`/`UserTagPage` already moved into
`posts/component.rs` with #323. They are ~95% posts-domain (they reuse
`PostCard`, `list_posts_by_tag` / `list_user_posts_by_tag`, `PageSeed`, posts
pagination) — structurally identical to `UserTimelinePage` beside them.
Maintainer decision (2026-07-24): **they stay in `posts/`**; only the
`Signal::derive` tidy is applied to them, in place.

## Goal

Give the tags vertical its own co-located UI home and satisfy the ADR-0070
floor, without disturbing the posts-timeline pages.

## Scope — in

1. **Relocate `TagInput` into a new wasm-only `tags/component.rs`.**
   - Verbatim move of the
     `#[component] pub fn TagInput(tags: RwSignal<Vec<TagSummary>>, #[prop(default = "tags")] name: &'static str)`
     from `posts/component.rs` into `tags/component.rs`. It is already `pub`.
   - `tags/component.rs` is declared
     `#[cfg(target_arch = "wasm32")] mod component;` in `tags/mod.rs` and
     carries **zero cfg gates inside the file** (ADR-0070 idiom).
   - `tags/mod.rs` gains `#[cfg(target_arch = "wasm32")] mod component;` and
     `#[cfg(target_arch = "wasm32")] pub use component::TagInput;`.
   - Rewire the 3 composer call sites in `posts/component.rs` (~576, ~701,
     ~1728) to `crate::tags::TagInput` (import `use crate::tags::TagInput;` and
     drop the now-moved local definition). Remove `TagInput` from the
     `posts/mod.rs` re-export.
   - The move carries `TagInput`'s tags-specific imports
     (`crate::tags::list_tags`, `common::tag::TagLabel`,
     `common::seed::TagSummary`, and whatever browser glue it already uses) into
     `component.rs`.

2. **Confirm `tags/api.rs` + `tags/mod.rs` are ADR-0070-canonical.**
   - `api.rs`: the `list_tags` endpoint stays here. Its whole body is a single
     inline `#[server]` fn (one `PostStorage` call + map); there are **no
     host-only support fns to hoist**, so **no `tags/server.rs` is created** —
     that matches ADR-0070 (`server.rs` exists only when there is support code).
     Keep the existing grouped `#[cfg(feature = "server")]` import block.
   - `mod.rs`: `mod api;` stays ungated (dual-compiled); add the component
     wiring from scope item 1. `mod.rs` stays wiring-only (no items of its own).

3. **Opportunistic `Signal::derive` collapse (from #505), in place in
   `posts/component.rs`:**
   - `SiteTagPage`:
     `Topbar title=Signal::derive(move || format!("#{}", read_tag()))` →
     `title=move || format!("#{}", read_tag())`.
   - `UserTagPage`: both `title` **and** `sub` `Signal::derive(move || …)`
     wrappers → bare `move || …` closures (mirroring how `UserTimelinePage`
     already spells it).
   - Behavior-identical; purely a consistency cleanup.

## Scope — out (recorded so it isn't re-litigated)

- **`SiteTagPage` / `UserTagPage` stay in `posts/`** (posts-timeline pages; see
  correction above). Only the `Signal::derive` tidy touches them.
- **`taglist/`** (the `TagList` footer-chip twin: `mod`/`markup`/`component`) is
  left untouched — ADR-0070 #527 deliberately makes it a **top-level
  shared-leaf** module (alongside `avatar`/`icon`/`topbar`), not part of any
  single vertical.
- **`TagCtx`** stays in `crate::render` (shared by the posts projector and
  `taglist`); moving it would ripple into `posts/render.rs` for no benefit here.
- No new tests beyond what compiles/relocates: `TagInput`'s behavior is
  unchanged and already exercised end-to-end; the endpoint's `TagLabel`/casing
  unit tests move with nothing (they stay in `tags/api.rs`).

## Acceptance

- `tags/` contains the tags UI (`TagInput` in `component.rs`), the `list_tags`
  `#[server]` fn + wire types (`api.rs`), wired by a wiring-only `mod.rs` — the
  vertical's UI + server fns + wire types are co-located. No `pages/tags.rs`
  exists (never did) and `TagInput` no longer lives in `posts/`.
- `posts/component.rs` imports `crate::tags::TagInput`; its 3 composer call
  sites compile against the relocated component; the stale local `TagInput`
  definition and the `posts/mod.rs` re-export are gone.
- The three `Signal::derive` wrappers on the tag pages are collapsed to bare
  closures; behavior unchanged.
- No fake-value host stub introduced (ADR-0055). No `target_arch` cfg appears
  inside any leaf file — only on the `mod component;` line (ADR-0070 §2; the
  wiring-only scan gate stays green).
- `cargo xtask validate` green, including wasm-clippy (`-p web`, wasm32 target)
  and this vertical's e2e flows (tag autocomplete in the composer; tag-timeline
  browsing).

## Risks / coordination

- **#643 (`posts: dissolve the SSR-era Resource→Effect indirections`)** is an
  in-flight branch that rewrites `posts/component.rs` (~200 lines, −138 net),
  targeting the Resource/Effect machinery in the timeline/tag **pages**. This
  issue's edits to that file are the `TagInput` extraction (a self-contained
  composer widget, away from the page machinery) and the `Signal::derive`
  collapse on the tag pages' `Topbar` lines (small, localized). Collision is
  bounded and sequence-tolerant: keep this branch current with `main`; whichever
  of #328/#643 merges second resolves the (small) overlap. Leaving the tag pages
  in `posts/` (rather than relocating them) is what keeps this collision small.
- `TagInput` is `#[component]`, so it moves to a **wasm-only** file. Verify it
  uses no host-only paths (it doesn't today) and run wasm-clippy before
  committing (host `cargo check`/`build` skip the wasm target and its
  `-D warnings` lints).

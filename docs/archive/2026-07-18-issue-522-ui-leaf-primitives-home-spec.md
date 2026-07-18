# Spec — issue #522: relocate shared leaf UI primitives to `web::ui`

- Issue: [#522](https://github.com/jaunder-org/jaunder/issues/522) — part of
  #303 (umbrella), the concrete "shared leaf primitives → a leaf home" increment
  of #312.
- ADR: [ADR-0056](../../adr/0056-web-canonical-colocated-leptos.md) (canonical
  co-located Leptos CSR; `#[component]` UI host-compiles ungated,
  coverage-exempt).
- Worktree: `.claude/worktrees/issue-522-ui-leaf-primitives-home`, branch
  `worktree-issue-522-ui-leaf-primitives-home`, fork tag `wt-base-issue-522`.

## Goal

Relocate the shared leaf UI primitives — **`Avatar`, `Icon`, `Chip`, `Dot`,
`TagList`** (plus the `Icons` glyph-data re-export) — from the wasm-gated
`web/src/pages/ui.rs` into the host-compiled `web::ui` home (`web/src/ui/`,
alongside `Topbar`). They are pure `#[component]`s with no browser API, so they
host-compile as-is (dual-target, coverage-exempt) with **no `target_arch` gate**
and **no fake-value host stub** (ADR-0055/0056 principle). This unblocks the
widgets that consume them — `PostCard`/`PostDisplay`/composer (posts #323) and
`Sidebar` (shell #330) — which cannot host-compile until the leaves already have
a host home.

## Current state (verified against source)

- `web/src/pages/ui.rs` is wasm-only via
  `#[cfg(target_arch = "wasm32")] pub mod pages;` at `web/src/lib.rs:31`.
  `web/src/ui/` is ungated/host-compiled (`lib.rs:44`).
- **`Topbar` is the established precedent**: its reactive `#[component]` and its
  pure projector twin `render_topbar` both live in `web/src/ui/topbar.rs`;
  `render/mod.rs` imports the twin back
  (`use crate::ui::topbar::render_topbar;`, `render/mod.rs:18`) and calls it
  mid-projector. `web/src/ui/mod.rs` documents the convention: _"Each widget's
  reactive `#[component]` (client) and its pure render twin (projector) live
  together in one file here (ADR-0056)."_
- The six primitives (all in `pages/ui.rs`): `TagList` (32–71), `Icon` (82–99),
  `Avatar` (103–122), `Dot` (126–135), `Chip` (139–153); `Icons` is only the
  re-export `pub use crate::render::Icons;` (78). **`Chip` renders `Dot`** — the
  only inter-primitive edge. None touches web-sys or any browser API; none has a
  `target_arch` gate or coverage marker.
- **Three pure twins** live in `web/src/render/mod.rs` and are consumed only
  inside that file's projector: `render_avatar` (107, `pub`, used @440),
  `render_icon` (614, `pub`, used by `render_sidebar` @641/653),
  `render_tag_list` (528, `pub(crate)`, used @494). Their dedicated
  markup-parity unit tests live in `render/mod.rs`'s `#[cfg(test)]` block
  (avatar @1139, tag_list @1153+). `Chip`, `Dot`, `Icon`-component have no
  twins.
- Shared pure helpers stay in `render`: `avatar_parts` (`pub`,
  `render/mod.rs:93`), `escape_html` (`pub(crate)`, 134), `Icons` struct (555),
  `TagCtx` enum (83), `NAV_ITEMS`/`SIDEBAR_SOURCES` — all used pervasively by
  the projector.
- Consumers of the six are **all inside the `web` crate**: the aggregate
  re-export `web/src/pages/mod.rs:16–18` (`Avatar, Chip, Dot, Icon, Icons`, not
  `TagList`); in-file `pages/ui.rs` uses (`Avatar` @306/617/1101, `Icon`
  @934/1048, `Icons::*` @1048/1072/1078, `Dot` inside `Chip`); `TagContext` (the
  `pub use render::TagCtx as TagContext` alias, `pages/ui.rs:27`) used by
  `ComposerFields`/`PostCard` and `pages/posts.rs`. `web/src/audiences/mod.rs`
  sources `Icons` from `render` directly and is unaffected.

## Scope decisions (approved in interview)

1. **Co-locate the pure twins with their components**, dropping the
   now-redundant noun from each name — the module carries it: `render_avatar` →
   `ui::avatar::render`, `render_icon` → `ui::icon::render`, `render_tag_list` →
   `ui::taglist::render`, each `pub(crate)` with its dedicated parity test.
   `render/mod.rs`'s projector **calls them directly at their new home** —
   `use crate::ui::{avatar, icon, taglist};` then `avatar::render(name, 38)`,
   `icon::render(path, 16)`, `taglist::render(tags, ctx)` — not aliased back
   under the old name. (This improves on the `render_topbar` precedent, which
   kept the old name; see the optional consistency tidy below.)
2. **One file per primitive** under `web/src/ui/`: `avatar.rs`, `icon.rs`,
   `chip.rs`, `dot.rs`, `taglist.rs` — matching `topbar.rs`'s "one widget per
   file".
3. **`escape_html` stays in `render`.** It is already host-compiled,
   `pub(crate)`, dependency-free, and has 16 in-`render` call sites plus
   `ui/topbar.rs` and the moved twins as consumers — the shared-pure-helper
   home. Moving it is neither required nor "convenient" (issue's optional item
   declined).
4. **`Icons` (struct) and `TagCtx` (enum) stay in `render`** — they are
   pervasive projector data/state, not leaf UI; `Icons` is surfaced from `ui` as
   a re-export, `TagCtx` continues to back the `TagContext` alias.

## Target layout

`web/src/ui/`:

- `avatar.rs` — `#[component] pub fn Avatar` (imports
  `crate::render::avatar_parts`)
  - `pub(crate) fn render` twin (imports
    `crate::render::{avatar_parts, escape_html}`)
  - avatar parity test. Twin markup kept byte-identical to the component.
- `icon.rs` — `#[component] pub fn Icon` + `pub(crate) fn render` twin
  (self-contained)
  - icon parity test; re-exports `Icons` for call sites
    (`pub use crate::render::Icons;`).
- `dot.rs` — `#[component] pub fn Dot`.
- `chip.rs` — `#[component] pub fn Chip` (uses `Dot` via `crate::ui::Dot`).
- `taglist.rs` — `#[component] pub fn TagList` (imports
  `crate::render::{TagCtx, TagSummary}`) + `pub(crate) fn render` twin +
  tag-list parity tests.
- `mod.rs` — add `pub mod {avatar, icon, chip, dot, taglist};` and
  `pub use {avatar::Avatar, icon::Icon, chip::Chip, dot::Dot, taglist::TagList, icon::Icons};`.

`web/src/render/mod.rs`:

- Delete the three twin definitions and their dedicated tests; add
  `use crate::ui::{avatar, icon, taglist};` and update the projector call sites
  to call the twins at their new home: `avatar::render(…)`, `icon::render(…)`,
  `taglist::render(…)` (was `render_avatar`/`render_icon`/`render_tag_list`).
- Update doc comments that name `pages::ui::{Avatar,Icon,TagList,TagContext}` to
  `ui::…` (e.g. `render/mod.rs:79, 90, 105, 526, 554, 580`).

**Consistency tidy (approved):** realign the pre-existing `Topbar` twin to the
same convention — `ui::topbar::render_topbar` → `ui::topbar::render`, updating
its `render/mod.rs` call sites (via the same `use crate::ui::topbar;`) and its
in-file test — so all four widget twins read `<widget>::render` rather than
leaving `topbar` the lone `render_*`-named outlier.

`web/src/pages/ui.rs`:

- Delete the six component definitions and the `Icons` re-export (78). Keep the
  `TagContext` alias (27) in place (non-moved `ComposerFields`/`PostCard` and
  `pages/posts.rs` consume it; it is a `render` type alias, not a component).
- Add a strangler shim mirroring the existing `pub use crate::ui::Topbar;`
  (157): `pub use crate::ui::{Avatar, Chip, Dot, Icon, Icons, TagList};` so the
  aggregate re-export at `pages/mod.rs:16–18` and the ~7 in-file consumers keep
  resolving.

`web/src/pages/mod.rs`: unchanged — its
`pub use ui::{Avatar, Chip, Dot, Icon, Icons, …}` resolves through the shim.

## Acceptance floor

- `Avatar`, `Icon`, `Chip`, `Dot`, `TagList` (+ `Icons` re-export) live in
  `web::ui`, host-compile (dual-target, coverage-exempt), and are importable by
  host-compiling verticals; their three twins co-locate with them.
- No `pages/ui.rs` **definition** of these primitives remains (re-export shim OK
  until #312 closes). No `target_arch` gate, no fake host stub added.
- Twin markup-parity unit tests pass in their new homes; projector output
  unchanged.
- `cargo xtask validate` green (host static + clippy + coverage; wasm-clippy;
  e2e — the reactive UI is unchanged behaviorally).

## Out of scope

- Moving `escape_html`, `Icons`, `TagCtx`, `avatar_parts` out of `render`.
- Relocating the widgets that _consume_ these leaves
  (`PostCard`/`Sidebar`/composer) — those are the posts #323 / shell #330
  verticals this issue unblocks.
- Deleting `pages/` and its module gate (the #312 cleanup that closes the
  umbrella).

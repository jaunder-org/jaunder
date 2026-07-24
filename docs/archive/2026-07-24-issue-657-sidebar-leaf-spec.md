# Spec — #657 web(sidebar): relocate the Sidebar to a co-located leaf home; delete `pages/ui.rs`

**Issue:** [#657](https://github.com/jaunder-org/jaunder/issues/657) — part of
#303; tracked by #312 (the `pages/ui.rs`-deleted half). **Unblocks #330.**
**Design floor:** ADR-0070 four-file split; the leaf-module twin pattern of
`web::topbar` (host `markup.rs` + wasm `component.rs` + wiring `mod.rs`).

## Context — the Sidebar cluster

`web/src/pages/ui.rs` is otherwise shim-only re-exports (`avatar::Avatar`,
`icon::{Icon, Icons}`, `taglist::TagList`, `topbar::Topbar`, and
`render::TagCtx as TagContext`) — **all vestigial**: no code imports
`crate::pages::ui::*`; the real consumers import from the leaf modules /
`web::render` directly (only doc comments in `render/mod.rs` and
`posts/render.rs` still name `pages::ui::…`).

The one piece of real UI is the **Sidebar cluster**, split across two modules:

- **Reactive** (`pages/ui.rs`): `Sidebar` (`#[component]`), private
  `SidebarNavItem`, `SidebarSource`, and `authed_sidebar` (currently
  `cov:ignore`'d, host-uncovered).
- **Pure twin** (`web::render`): `render_sidebar` (the anonymous projector),
  plus the sidebar-only consts `NAV_ITEMS` and `SIDEBAR_SOURCES` (consumed by
  _both_ `render_sidebar` and `authed_sidebar`).

`Sidebar`'s anon path calls `render::render_sidebar` (inner_html, flash-free
coincidence); `render_shell` also embeds `render_sidebar("")`. Per #312's rule —
"reactive `#[component]`

- pure render twin migrate as one self-contained unit" — the whole cluster moves
  together to a new leaf home.

## Scope

### New `web::sidebar` leaf module (mirrors `web::topbar`)

- **`web/src/sidebar/markup.rs`** (host-compiled, pure): move `render_sidebar` +
  `NAV_ITEMS`
  - `SIDEBAR_SOURCES` + their host tests out of `web/src/render/mod.rs`. The
    consts become `pub(crate)`/module-internal (they were `pub` in `render` only
    to reach `authed_sidebar`; now co-located). **Bring the file-scope imports
    these relied on in `render/mod.rs`** — at minimum
    `use std::fmt::Write as _;` and `use crate::icon::Icons;` — a "verbatim"
    move won't compile without them (the gate confirms the exact set).
- **`web/src/sidebar/component.rs`** (wasm-only): move `Sidebar` +
  `SidebarNavItem` + `SidebarSource` + `authed_sidebar` from `pages/ui.rs`.
  Declared `#[cfg(target_arch = "wasm32")] mod component;`. **Zero `#[cfg(...)]`
  inside; drop the `cov:ignore` block on `authed_sidebar`** (wasm-only ⇒
  coverage-exempt wholesale; note `pages` is already `target_arch`-gated so this
  `cov:ignore` is redundant today — the drop is a cleanup, not a coverage
  change). Imports: `leptos::prelude::*`, `crate::icon::{Icon, Icons}`,
  `crate::avatar::Avatar`, `crate::auth::use_session`,
  `super::markup::{render_sidebar, NAV_ITEMS, SIDEBAR_SOURCES}`,
  `common::username::Username`.
- **`web/src/sidebar/mod.rs`** (wiring only): `mod markup;` +
  `pub(crate) use markup::render_sidebar;` (for `render_shell` — same-crate
  only, matching `topbar`'s `pub(crate) use markup::render`);
  `#[cfg(target_arch = "wasm32")] mod component;`
  - `#[cfg(target_arch = "wasm32")] pub use component::Sidebar;`.
- Declare `pub mod sidebar;` in `lib.rs` (ungated leaf, internal gating — like
  `topbar`).

### Repoint the two consumers

- **`web/src/render/mod.rs`**: `render_shell` calls
  `crate::sidebar::render_sidebar("")` instead of the local fn. Remove
  `render_sidebar`, `NAV_ITEMS`, `SIDEBAR_SOURCES`, and their tests from this
  file (they moved to `sidebar/markup.rs`). Update the doc comments that name
  `pages::ui::Sidebar` to `crate::sidebar::Sidebar`.
- **`web/src/pages/mod.rs`**: `AppShell` renders `crate::sidebar::Sidebar`.
  Remove `pub mod ui;` and
  `pub use ui::{Avatar, Icon, Icons, Sidebar, Topbar};`.

### Delete `pages/ui.rs`

Its only real code (the Sidebar cluster) has moved; its re-exports are
unconsumed. Delete the file. Update **every** stray `pages::ui::…` doc-comment
reference to the real home — they live in `posts/render.rs` (a `PostDisplay`
mention) and `render/mod.rs` (`pages::ui::Sidebar`, `pages::ui::TagContext`) →
`crate::sidebar::Sidebar` / `crate::render::TagCtx` respectively (`TagCtx`
itself stays in `web::render` for #658).

## Acceptance criteria

- **AC1** `web/src/sidebar/` exists as `mod.rs`/`markup.rs`/`component.rs`.
  `markup.rs` (host) holds `render_sidebar` + the two consts + their tests;
  `component.rs` (wasm-only) holds `Sidebar` + `SidebarNavItem` +
  `SidebarSource` + `authed_sidebar`, with **no `#[cfg(...)]` line and no
  `cov:ignore`**. `pub mod sidebar;` declared in `lib.rs`.
- **AC2** `web/src/render/mod.rs` no longer defines
  `render_sidebar`/`NAV_ITEMS`/ `SIDEBAR_SOURCES`; `render_shell` calls
  `crate::sidebar::render_sidebar`. The moved host tests pass in their new home.
- **AC3** `web/src/pages/ui.rs` does not exist; `pages/mod.rs` no longer
  declares `pub mod ui;`/`pub use ui::{…}`; `AppShell` renders
  `crate::sidebar::Sidebar`. `rg 'pages::ui' web/src` yields nothing (doc
  comments updated).
- **AC4** The only `target_arch` gates in `web/src/sidebar` are the
  `mod component;` declaration and its `pub use` re-export. No fake host stub
  (ADR-0055) — `component.rs` is wasm-only, never host-compiled.
- **AC5** No behavior change: `cargo xtask validate` green including the e2e
  matrix. The sidebar renders identically — anon (inner_html of
  `render_sidebar`, flash-free) and authed (`authed_sidebar`) — exercised by the
  existing shell/theme/visibility e2e; the `render_sidebar` host tests
  (brand/nav/sources/active-class) still pass.

## Out of scope

- The rest of `web::render` (`render_shell`, `render_head`, `escape_html`,
  masthead, etc.) — that is #658. This issue only extracts `render_sidebar` +
  the sidebar consts (the Sidebar's twin) and leaves `render_shell` calling into
  `web::sidebar`.
- `App`/Router relocation — #330.
- `TagCtx` relocation — stays in `web::render` for #658; this issue only drops
  the unconsumed `pages/ui.rs` re-export of it.

## Decisions / ADRs

No new ADR. A leaf-module relocation within the established ADR-0070 /
`web::topbar` pattern.

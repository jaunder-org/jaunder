# Plan — issue #522: relocate shared leaf UI primitives to `web::ui`

Spec:
[`2026-07-18-issue-522-ui-leaf-primitives-home.md`](../specs/2026-07-18-issue-522-ui-leaf-primitives-home.md)
(the "what/why"; this plan is the "how"). Issue
[#522](https://github.com/jaunder-org/jaunder/issues/522).

## Review header

**Goal.** Move the five leaf `#[component]`s (`Avatar`, `Icon`, `Chip`, `Dot`,
`TagList`) + the `Icons` re-export from the wasm-gated `web/src/pages/ui.rs`
into the host-compiled `web/src/ui/` home (one file per primitive, `Topbar` is
the template). Co-locate each component's pure projector twin, renamed
`<widget>::render`, and have `render/mod.rs` call the twins at their new home.
Leave strangler re-export shims in `pages/ui.rs`.

**Scope — in:** `web/src/ui/{avatar,icon,chip,dot,taglist}.rs` (new),
`web/src/ui/mod.rs`, `web/src/ui/topbar.rs` (twin rename),
`web/src/render/mod.rs` (delete 3 twins + their tests, call twins at new home,
doc-comment fixups), `web/src/pages/ui.rs` (delete 5 defs + `Icons` re-export,
add shim). **Out:** `escape_html`/`Icons`/`TagCtx`/ `avatar_parts` stay in
`render`; the consumer widgets (`PostCard`/`Sidebar`/composer) and the `pages/`
deletion are #323/#330/#312.

**Tasks (one commit each):**

1. Move `Dot` + `Chip` → `ui/dot.rs`, `ui/chip.rs` (no twins; `Chip` uses
   `ui::Dot`).
2. Move `Avatar` + its twin → `ui/avatar.rs` (`avatar::render` + parity test).
3. Move `Icon` + its twin + `Icons` re-export → `ui/icon.rs` (`icon::render` +
   test).
4. Move `TagList` + its twin → `ui/taglist.rs` (`taglist::render` + 3 parity
   tests).
5. Rename the `Topbar` twin `render_topbar` → `topbar::render` (consistency).
6. Doc-comment sweep in `render/mod.rs` + full `cargo xtask validate`.

**Key risks / decisions.**

- **Coverage-neutral:** the twins are coverage-_measured_ plain fns; each moves
  **with its dedicated test** into `ui/` (also measured) — never move a twin
  without its test. The `#[component]`s are syntactically coverage-exempt in any
  module (ADR-0056), so host-compiling them adds no debt. No `cov:ignore`, no
  `target_arch` gate, no host stub.
- **`render`↔`ui` mutual module imports** are already proven by `Topbar`; no
  circular _crate_ dep. `pages/ui.rs` is wasm-only, so its shim + in-file
  consumers only exist on wasm; host consumers use `crate::ui::` — no host
  breakage.
- **Twin ⇔ component markup parity** is load-bearing (seeded paint must equal
  reactive re-render). Move twin bodies **byte-for-byte**; the parity tests
  guard it.
- **leptosfmt gotcha:** keep intent comments _outside_ `view!` macros on the
  moved components (it relocates block comments near `return`/inside the macro).

**Global constraints.**

- Rust; run the gate via `devtool run -- cargo xtask check` (worktree-aware,
  honest exit); read the parked log, don't pipe. `validate` foreground with
  `timeout: 600000`.
- Commit per task via **`jaunder-commit`** (pre-commit hook runs full
  `cargo xtask check`). **No `Co-Authored-By` trailer.** Serialize
  edit→gate→commit (no edits during a gated commit).
- Each moved item keeps its existing attributes verbatim
  (`#[expect(clippy:: needless_pass_by_value, …)]` on `Avatar`/`Dot`/`TagList`;
  `#[must_use]` on twins) and its doc comment (relocated into the new file,
  intra-doc links repointed).

**For agentic workers.** Execute with **`jaunder-iterate`**, delegating a task
to **`jaunder-dispatch`** when useful; tick checkboxes in real time. Do not
start before the plan-approval HALT.

---

## Task 1 — `Dot` + `Chip` → `ui/dot.rs`, `ui/chip.rs`

Establishes the file + shim + `mod.rs` pattern with the twin-less coupled pair.

**Files**

- New `web/src/ui/dot.rs`: `//!` module doc; `use leptos::prelude::*;`; move
  `#[component] pub fn Dot` (`pages/ui.rs:126-135`, incl. its `#[expect(…)]` and
  doc) verbatim.
- New `web/src/ui/chip.rs`: `//!` doc; `use leptos::prelude::*;`;
  `use crate::ui::Dot;`; move `#[component] pub fn Chip` (`pages/ui.rs:139-153`)
  verbatim; the `<Dot proto=p/>` call now resolves via the import.
- `web/src/ui/mod.rs`: add `pub mod dot;`, `pub mod chip;`; add
  `pub use dot::Dot;`, `pub use chip::Chip;`.
- `web/src/pages/ui.rs`: delete the `Dot` and `Chip` definitions; add (next to
  the existing `pub use crate::ui::Topbar;` at line 157)
  `pub use crate::ui::{Chip, Dot};`.

**Interfaces** — component signatures unchanged (`Dot(proto: String)`,
`Chip(label, proto, count, active)`). `pages/mod.rs:16-18` aggregate re-export
(`…Chip, Dot…`) resolves through the shim — unchanged. In-file `<Dot>` inside
`Chip` now lives in `ui`; no other in-`pages` consumer of `Dot`/`Chip`.

**Steps**

1. Create `ui/dot.rs`, `ui/chip.rs`; wire `ui/mod.rs`.
2. Delete defs from `pages/ui.rs`; add the shim line.
3. `devtool run -- cargo xtask check` → green (host static + clippy +
   wasm-clippy + coverage). Fixes: formatting auto-applied; confirm no leptosfmt
   comment relocation.
4. Commit (**jaunder-commit**):
   `web(ui): relocate Dot and Chip to web::ui (#522)`.

**Verify** — `check` green; `rg -n 'fn Dot|fn Chip' web/src/pages/ui.rs` returns
only the shim `pub use`;
`rg -n 'pub use crate::ui::\{?Chip' web/src/pages/ui.rs` present.

## Task 2 — `Avatar` (+ twin) → `ui/avatar.rs`

**Files**

- New `web/src/ui/avatar.rs`: `//!` doc; `use leptos::prelude::*;`;
  `use crate::render::{avatar_parts, escape_html};`
  - move `#[component] pub fn Avatar` (`pages/ui.rs:103-122`, incl.
    `#[expect(…)]` and the "byte-identical to render_avatar" comment, repointed
    to `render`);
  - move the twin as
    `#[must_use] pub(crate) fn render(name: &str, size: u32) -> String` (body
    byte-identical to `render_avatar`, `render/mod.rs:107-116`);
  - `#[cfg(test)] mod tests` with the moved
    `avatar_matches_reactive_component_markup` test (`render/mod.rs:1139-1151`),
    calling `super::render` and `crate::render::avatar_parts`.
- `web/src/ui/mod.rs`: add `pub mod avatar;`, `pub use avatar::Avatar;`.
- `web/src/render/mod.rs`:
  - delete `render_avatar` (107-116) and its test (1139-1151);
  - add `use crate::ui::avatar;` (or extend a combined `use crate::ui::{…};`);
  - update the call site at ~440 `render_avatar(view.username, 38)` →
    `avatar::render(view.username, 38)`;
  - repoint the intra-doc link on `avatar_parts` (~91) and the doc at ~104-105
    (`pages::ui::Avatar` → `ui::Avatar`, `render_avatar` → `avatar::render`).
- `web/src/pages/ui.rs`: delete `Avatar` def; extend shim →
  `pub use crate::ui:: {Avatar, Chip, Dot};`. In-file `<Avatar>` uses
  (306/617/1101) resolve via shim.

**Steps** — create file; edit `render/mod.rs`; edit `pages/ui.rs` + `ui/mod.rs`;
`devtool run -- cargo xtask check` → green (the moved parity test runs in `ui`);
commit `web(ui): relocate Avatar + render twin to web::ui (#522)`.

**Verify** — `cargo nextest run -p web avatar_matches_reactive_component_markup`
PASS; `rg -n 'render_avatar' web/src` returns nothing; `check` green.

## Task 3 — `Icon` (+ twin + `Icons` re-export) → `ui/icon.rs`

**Files**

- New `web/src/ui/icon.rs`: `//!` doc; `use leptos::prelude::*;`;
  `pub use crate::render::Icons;` (the glyph-data re-export; `Icons` struct
  stays in `render`);
  - move `#[component] pub fn Icon` (`pages/ui.rs:82-99`, incl. doc repointed);
  - move the twin as
    `#[must_use] pub(crate) fn render(path: &str, size: u32) -> String`
    (byte-identical to `render_icon`, `render/mod.rs:614-624`);
  - `#[cfg(test)] mod tests` with the moved
    `icon_matches_reactive_component_markup` test (`render/mod.rs:1307-1318`),
    calling `super::{render, Icons}`.
- `web/src/ui/mod.rs`: add `pub mod icon;`, `pub use icon::{Icon, Icons};`.
- `web/src/render/mod.rs`:
  - delete `render_icon` (614-624) and its dedicated test;
  - add `use crate::ui::icon;`;
  - update `render_sidebar` call sites at ~641 and ~653 `render_icon(…)` →
    `icon::render(…)`; keep `Icons::SEARCH`/`NAV_ITEMS` (still local);
  - repoint the doc at ~554 (`pages::ui::Icon` → `ui::Icon`, `render_icon` →
    `icon::render`).
- `web/src/pages/ui.rs`: delete `Icon` def and the
  `pub use crate::render::Icons;` line (78); extend shim →
  `pub use crate::ui::{Avatar, Chip, Dot, Icon, Icons};`. In-file
  `<Icon>`/`Icons::*` uses (934/1048/1072/1078) resolve via shim.

**Steps** — as Task 2; `check` green; commit
`web(ui): relocate Icon + render twin and Icons re-export to web::ui (#522)`.

**Verify** — the moved icon test PASS; `rg -n 'render_icon' web/src` empty;
`rg -n 'crate::render::Icons' web/src/pages/ui.rs` empty; `check` green.

## Task 4 — `TagList` (+ twin) → `ui/taglist.rs`

**Files**

- New `web/src/ui/taglist.rs`: `//!` doc; `use leptos::prelude::*;`;
  `use crate::render::TagCtx;`, `use crate::tags::TagSummary;`
  - move `#[component] pub fn TagList` (`pages/ui.rs:32-71`, incl.
    `#[expect(…)]` and doc) verbatim; its `context: TagContext` param stays
    typed `TagContext` — add a local `use crate::render::TagCtx as TagContext;`
    in `taglist.rs` **or** change the signature to `TagCtx` directly (prefer the
    latter for clarity; the alias remains in `pages/ui.rs` for other consumers).
    Confirm callers still compile.
  - move the twin as
    `#[must_use] pub(crate) fn render(tags: &[TagSummary], ctx: &TagCtx) -> String`
    (byte-identical to `render_tag_list`, `render/mod.rs:528-551`);
  - `#[cfg(test)] mod tests` with the 3 moved tests (`render/mod.rs:1153-1188`:
    `tag_list_site_wide_has_hash_chip_and_no_here_link`,
    `tag_list_for_user_adds_here_link`, `empty_tag_list_renders_nothing`),
    calling `super::render`, `crate::render::TagCtx`, `crate::tags::TagSummary`,
    `common::username::Username`.
- `web/src/ui/mod.rs`: add `pub mod taglist;`, `pub use taglist::TagList;`.
- `web/src/render/mod.rs`:
  - delete `render_tag_list` (528-551) and the 3 tests (1153-1188);
  - add `use crate::ui::taglist;`;
  - update the call site at ~494 `render_tag_list(view.tags, view.tag_ctx)` →
    `taglist::render(view.tags, view.tag_ctx)`; `TagCtx` stays local;
  - repoint docs at ~79 and ~526 (`pages::ui::{TagContext,TagList}` → `ui::…`).
- `web/src/pages/ui.rs`: delete `TagList` def; extend shim →
  `pub use crate::ui:: {Avatar, Chip, Dot, Icon, Icons, TagList};`. Keep the
  `TagContext` alias (27) — it is consumed by `ComposerFields`/`PostCard`
  (264/331) and `pages/posts.rs`.

**Steps** — as before; `check` green; commit
`web(ui): relocate TagList + render twin to web::ui (#522)`.

**Verify** — the 3 tag-list tests PASS in `web`;
`rg -n 'render_tag_list' web/src` empty; `TagContext` still resolves for
`pages/posts.rs`; `check` green.

## Task 5 — `Topbar` twin rename `render_topbar` → `topbar::render`

Consistency tidy (approved) so all four widget twins read `<widget>::render`.

**Files**

- `web/src/ui/topbar.rs`: rename `pub(crate) fn render_topbar` →
  `pub(crate) fn render` (11); update the in-file test import
  (`use super::render_topbar;` → `use super::render;`) and its calls (53/65);
  update the doc/`[render_topbar]` link (25).
- `web/src/render/mod.rs`: replace `use crate::ui::topbar::render_topbar;` (18)
  with `use crate::ui::topbar;` (fold into the combined
  `use crate::ui::{avatar, icon, taglist, topbar};`); update the four call sites
  (289/310/317/328) `render_topbar(…)` → `topbar::render(…)`.

**Steps** — edit; `check` green; commit
`web(ui): rename Topbar twin to topbar::render for twin-naming consistency (#522)`.

**Verify** — `rg -n 'render_topbar' web/src` empty; the topbar in-file tests
PASS; `check` green.

## Task 6 — doc sweep + full validate

**Files** — `web/src/render/mod.rs`: sweep any remaining `pages::ui::` mentions
of the moved items in doc comments to `ui::…` (verify none of the _moved_ items
are still referenced under the old path; `pages::ui::Sidebar` at ~580 stays —
`Sidebar` is not in scope). Consolidate the
`use crate::ui::{avatar, icon, taglist, topbar};` import.

**Steps**

1. `rg -n 'pages::ui::(Avatar|Icon|Chip|Dot|TagList|TagContext|render_)' web/src`
   → fix any stragglers.
2. `devtool run -- cargo xtask validate` (foreground, `timeout: 600000`) →
   green: host static + clippy + coverage + e2e
   (`{sqlite,postgres}×{chromium,firefox}`). The reactive UI is behaviorally
   unchanged, so e2e should pass without spec edits.
3. If Task 1-5 commits need touch-ups surfaced here, fold them into the owning
   commit via `git commit --fixup` + autosquash (clean history), not a churn
   commit.
4. Commit any doc-only remainder:
   `web(ui): repoint render doc links after leaf move (#522)`.

**Verify** — `validate` green; acceptance floor met: the five primitives +
`Icons` live in `web::ui`, host-compile coverage-exempt; no primitive
**definition** remains in `pages/ui.rs` (shims only); the four twins read
`<widget>::render`; no `target_arch` gate or host stub added.

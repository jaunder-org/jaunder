# Spec — #658 web(render): dissolve `web::render` onto co-located homes

**Issue:** [#658](https://github.com/jaunder-org/jaunder/issues/658) — closes
the residual `web::render` half of #312 after #330 moved the app-shell
projector. **Worktree:** `.claude/worktrees/issue-658-render-leaf-homes` /
branch `worktree-issue-658-render-leaf-homes`. **Design floor:** ADR-0070
file-level host/wasm split; ADR-0055 retained rule that pure host-testable logic
stays ungated and no fake host stubs are introduced.

## Current state

`web::render` is no longer the app-shell projector. #330 already moved
`render_shell`, `render_head`, `DEFAULT_THEME`, `PREPAINT_SCRIPT`, `SPA_SHELL`,
and `DISCOVERY_MARKER_ATTR` to `web::app::render`.

The remaining `web/src/render/mod.rs` is a grab-bag of seven residual
primitives:

- `TagCtx`
- `escape_html`
- `render_hero`
- `render_home_masthead`
- `render_load_more`
- `Icons`
- `format_bytes`

The deletion target is therefore narrower than issue #658's original text: the
shell half is already done; this cycle owns only the residual primitives above
and the final deletion of `web/src/render/` + `pub mod render`.

## Destination map

### Shared low-level HTML primitive

Create `web/src/html.rs`:

- Move `escape_html` and its test here.
- Keep it ungated, host-compiled, and `pub(crate)`.
- Add `mod html;` in `web/src/lib.rs` so crate-internal callers can use
  `crate::html::escape_html`.
- Repoint `app::render`, `avatar::markup`, `posts::render`, `taglist::markup`,
  and `topbar::markup` to `crate::html::escape_html`.

Rationale: escaping is deliberately cross-cutting and not owned by a widget or
vertical. A tiny `html` leaf is deeper than leaving every caller to own escaping
or placing it under an unrelated widget.

### Icon glyph paths

Create `web/src/icon/paths.rs`:

- Move `Icons` into the `icon` leaf.
- Update `web/src/icon/mod.rs` to declare `mod paths;` and
  `pub use paths::Icons;`.
- Repoint direct consumers to `crate::icon::Icons`.
- Update `icon::markup` tests to import from `super`/`crate::icon`, not
  `crate::render`.

Rationale: `Icons` is the interface shared by the reactive `Icon` and pure
`icon::render` twin. The icon module already is the top-level shared leaf home
ADR-0070 prescribes.

### Tag-list linking context

Create `web/src/taglist/context.rs`:

- Move `TagCtx` into the taglist leaf.
- Update `web/src/taglist/mod.rs` to declare `mod context;` and
  `pub use context::TagCtx;`.
- Repoint `posts::{component,render}`, `timeline::component`, and taglist tests
  to `crate::taglist::TagCtx` (or local `TagContext` aliases where they improve
  prop readability).

Rationale: `TagCtx` is not a general renderer context; it is the interface for
tag-list link behavior. Placing it beside `TagList` and `taglist::render` keeps
the caller-visible contract with the module that implements it.

### Home masthead

Create `web/src/home/render.rs`:

- Move `render_hero` and `render_home_masthead` and the masthead test here.
- Rename the public-in-crate entry point to `render_masthead`; keep
  `render_hero` private to the file.
- Update `home::component` and `posts::render` to call
  `crate::home::render::render_masthead()`.
- Update `home/mod.rs` to declare `pub(crate) mod render;` plus the existing
  wasm-gated component wiring.

Rationale: the masthead is the pure twin consumed by the home vertical and the
projector's site timeline body. The module path should say that:
`home::render::render_masthead`, not global `render::render_home_masthead`.

### Timeline load-more placeholder

Create `web/src/timeline/render.rs`:

- Move `render_load_more` here.
- Add a focused host unit test in `timeline::render` for both `has_more`
  branches; keep the existing `posts::render` body-level test as composed
  coverage.
- Update `posts::render` to call `crate::timeline::render::render_load_more`.
- Update `timeline/mod.rs` to declare `pub(crate) mod render;`.

Rationale: the placeholder is the pure twin of the reactive timeline load-more
button in `timeline::component`, even though the projector calls it from
`posts::render`.

### Media byte-size formatter

Create `web/src/media/format.rs`:

- Move `format_bytes` and its four tests here.
- Keep the existing `#[expect(clippy::cast_precision_loss, …)]` with the same
  reason.
- Update `media/mod.rs` to declare `mod format;` and
  `pub use format::format_bytes;` — amended from the drafted `pub(crate)`, which
  does not compile: `format_bytes`'s only caller is the wasm-only
  `media::component`, so on the host build a crate-internal item is unreachable
  and clippy fails with `dead_code` + `unused_imports` under `-D warnings`.
  `pub` is the visibility the item already had as `web::render::format_bytes`,
  so reachability is unchanged and no suppression is needed.
- Update `media/component.rs` to use `super::format_bytes`.

Rationale: byte-size formatting is media display logic, not a renderer
primitive. Keeping it under `media` preserves host test coverage while moving
behavior to its primary consumer.

## Required cutover

- Delete `web/src/render/mod.rs`.
- Remove `pub mod render;` from `web/src/lib.rs`.
- Leave no compatibility shim or re-export from `web::render`; the issue's
  deliverable is elimination, not deprecation.
- Update active code comments and docs that describe live code paths as
  `web::render`/`crate::render`. Historical ADR/archive references may remain
  when they describe the architecture at that time.
- Preserve ADR-0070 wiring rules: `mod.rs` files contain declarations and
  re-exports only; no moved function bodies land in `mod.rs`.
- Preserve ADR-0055: no fake host stubs; every moved pure helper remains ungated
  and host-tested.

## Acceptance criteria

- **AC1 — render module gone.** `web/src/render/` does not exist and
  `web/src/lib.rs` has no `pub mod render;`.
- **AC2 — no stale call sites.**
  `rg 'crate::render|web::render|render::Icons|render::TagCtx' web/src` returns
  no live code references. Remaining `render` module references are co-located
  homes such as `crate::posts::render`, `crate::app::render`,
  `crate::home::render`, or `crate::timeline::render`.
- **AC3 — helpers live beside owners.** `escape_html` is in `web::html`; `Icons`
  in `web::icon`; `TagCtx` in `web::taglist`; masthead in `web::home::render`;
  load-more placeholder in `web::timeline::render`; `format_bytes` in
  `web::media`.
- **AC4 — host coverage preserved.** The tests currently in `web::render` move
  with their subjects, and the existing projector/body tests that cover composed
  behavior still pass.
- **AC5 — module-gating invariant preserved.** No new `target_arch` cfgs are
  introduced except existing wasm-only `component` module
  declarations/re-exports; moved pure files are ungated.
- **AC6 — no behavior change.** Generated shell/body/sidebar/post/home/media
  markup stays byte-equivalent except for module paths in Rust code/comments.
  Existing e2e flows that render the projector shell, home masthead, timeline
  load-more placeholder, icons, tag lists, avatars, and media quota labels
  continue to pass.
- **AC7 — gate green.** `cargo xtask validate` is green before PR handoff.

## Out of scope

- Changing the projector architecture from ADR-0041.
- Moving `app::render` shell code again; #330 already co-located it with
  `App`/`AppShell`.
- Introducing a general UI/shared namespace.
- Changing icon path data, tag link behavior, media quota math, masthead copy,
  or load-more markup.
- Editing historical ADRs or archived plans merely because they mention the old
  `web::render` architecture.

## Decisions / ADRs

No new ADR is required. This applies ADR-0070 and ADR-0055 to finish the
migration already recorded by #312/#330/#658.

## Open coordination note

The Jaunder Backlog claim could not be written in this session because
`gh project item-list` failed with missing `read:project` scope. The worktree
branch is created and unambiguous; project Status still needs to be set to **In
Progress** by a token with project scopes.

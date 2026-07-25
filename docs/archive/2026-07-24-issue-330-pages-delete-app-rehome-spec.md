# Spec — #330: delete `pages/`; rehome `App`/Router + the pure shell projector into `web::app`

**Issue:** jaunder-org/jaunder#330 (milestone _Web: canonical Leptos CSR
convergence_; umbrella #303). **Design records:** ADR-0056 →superseded by the
`web-vertical-wasm-only-component-files` draft (#526); ADR-0070 (four-file
vertical layout); ADR-0041 (projector↔reactive coincidence); ADR-0044 (authed
pre-paint); #520 (`target_arch` only on `mod`); #312 (a component migrates
**with its pure render twin as one unit**).

## Goal

The final convergence step. Every vertical has co-located, and after #304
(dropped `pages/signal_read.rs`), #657 (Sidebar → `web::sidebar`, `pages/ui.rs`
deleted), and #328 (last un-converged vertical, tags — **closed**),
`web/src/pages/` is reduced to a single `mod.rs`: `App` + `AppShell` + the
Router/route table.

This issue **deletes `pages/`** by relocating that shell — and, per #312's rule,
its **pure render twin** (the shell projector currently in `web::render`) — into
a new **`web::app`** vertical in the ADR-0070/#526 file layout. It is a
**faithful relocation**: behaviour-identical, no renames or restructuring beyond
what the host/wasm file split forces.

## Decisions (resolved in the design interview)

1. **New home = `web/src/app/`** (module `web::app`), three files per ADR-0070:
   `mod.rs` (wiring only), `render.rs` (pure host-compiled projector leaf),
   `component.rs` (wasm-only `#[component]` UI).
2. **Canonical `web::app::App`** — the crate-root `pub use pages::App` re-export
   is deleted outright (no root shim); the one external consumer (`csr`) imports
   `web::app::App`.
3. **Faithful move only** — no opportunistic cleanup within the shell.

## Module layout — `web/src/app/`

### `app/render.rs` — pure shell projector (ungated, host-compiled, host-tested)

An ADR-0070 "extra leaf" exactly like `web/src/posts/render.rs`: non-reactive,
plain-string HTML, `#[cfg]`-free, host-compiled and coverage-measured. Relocated
verbatim from `web::render`:

- Functions: `render_shell`, `render_head`, and its private helpers
  `render_discovery`, `feed_label`.
- Constants: `DEFAULT_THEME`, `PREPAINT_SCRIPT`, `SPA_SHELL`,
  `DISCOVERY_MARKER_ATTR`.
- The host tests for the above (the shell half of `web::render`'s current
  `#[cfg(test)] mod tests`). The `mod tests` is **split, not moved** — see the
  test partition below.

It continues to call the **residual** `web::render` leaves cross-module —
`crate::render::escape_html` (from `render_head`/`render_discovery`) and
`crate::posts::render::render_body` + `crate::sidebar::render_sidebar` (from
`render_shell`) — the same idiom `posts/render.rs` already uses (it imports
`crate::render::{escape_html, render_load_more, TagCtx}`). These cross-calls are
temporary; #658 dissolves the residual `web::render`.

### `app/component.rs` — wasm UI (`#[cfg(target_arch = "wasm32")] mod component;`)

Relocated verbatim from `pages/mod.rs`: `App` (pub `#[component]`), `AppShell`
(private `#[component]`), and the `THEME_KEY` const. No `#[cfg]` inside the file
(wasm-only via its `mod` line). References `DEFAULT_THEME` from the sibling leaf
(`super::DEFAULT_THEME` via the `mod.rs` re-export) instead of the former
`pub use crate::render::DEFAULT_THEME`.

### `app/mod.rs` — wiring only (no items of its own)

- `mod render;` +
  `pub use render::{render_shell, render_head, DEFAULT_THEME, PREPAINT_SCRIPT, SPA_SHELL, DISCOVERY_MARKER_ATTR};`
- `#[cfg(target_arch = "wasm32")] mod component;` +
  `#[cfg(target_arch = "wasm32")] pub use component::App;`

The only `target_arch` gates are on `mod` declarations (satisfies #520).

### Test partition (the shared `#[cfg(test)] mod tests` is split)

`render/mod.rs`'s test module currently holds 15 tests covering both halves.
Split by which symbol each exercises:

- **Move to `app/render.rs` tests (9)** — plus the
  `use crate::posts::render::test_fixtures::{one_post_page, sample_post}` and
  `use common::test_support::parse_username` imports (only these tests use
  them): `discovery_links_carry_the_marker_per_surface`,
  `default_theme_is_nonempty`,
  `prepaint_script_is_inline_blocking_and_reads_the_marker`,
  `index_html_shell_contains_the_prepaint_script`,
  `csr_index_html_boots_wasm_with_an_explicit_url`,
  `permalink_head_sets_escaped_title_and_og`,
  `head_titles_cover_every_page_kind`,
  `shell_wraps_body_in_j_root_with_sidebar_and_main`,
  `page_seed_round_trips_through_json` (tests the `PageSeed` the shell renders).
- **Stay in `render/mod.rs` tests (6)** — exercise residual symbols, need no
  fixture imports: the four `format_bytes_*`,
  `escape_replaces_markup_metacharacters` (residual `escape_html`),
  `home_masthead_has_topbar_hero_and_anon_only_cta` (residual
  `render_home_masthead`).

## `web/src/lib.rs`

- **Remove** `#[cfg(target_arch = "wasm32")] pub mod pages;` and
  `#[cfg(target_arch = "wasm32")] pub use pages::App;`.
- **Add** `pub mod app;` (ungated — the module has a host-compiled `render`
  leaf).
- `pub mod render;` **stays** (residual half lives until #658).

## Consumer ripple (`web::render::X` → `web::app::X`)

| File                                     | Current                                                          | New                               |
| ---------------------------------------- | ---------------------------------------------------------------- | --------------------------------- |
| `server/src/projector/mod.rs:44`         | `use web::render::{render_head, render_shell, PREPAINT_SCRIPT};` | `use web::app::{…};`              |
| `server/src/lib.rs:120`                  | `web::render::SPA_SHELL`                                         | `web::app::SPA_SHELL`             |
| `server/src/site.rs:141`                 | `web::render::SPA_SHELL`                                         | `web::app::SPA_SHELL`             |
| `server/src/projector/mod.rs:392` (test) | `web::render::PREPAINT_SCRIPT`                                   | `web::app::PREPAINT_SCRIPT`       |
| `csr/src/lib.rs:8`                       | `use web::App;`                                                  | `use web::app::App;`              |
| `csr/src/lib.rs:42`                      | `web::render::DISCOVERY_MARKER_ATTR`                             | `web::app::DISCOVERY_MARKER_ATTR` |
| `csr/src/lib.rs:2` (comment)             | `web::App's ParentRoute…`                                        | `web::app::App's ParentRoute…`    |
| `server/src/lib.rs:1` (comment)          | `web::App`                                                       | `web::app::App`                   |
| `web/src/posts/render.rs:33` (doc)       | ``[`crate::render::render_shell`]``                              | ``[`crate::app::render_shell`]``  |
| `web/src/auth/marker.rs:5` (doc)         | ``(`render::PREPAINT_SCRIPT`)``                                  | ``(`app::PREPAINT_SCRIPT`)``      |

`web/src/home/component.rs:69` calls `crate::render::render_home_masthead()` —
**unchanged** (`render_home_masthead` is residual, stays in `web::render`).

### Stale comments/doc-strings my move invalidates (also update)

The relocation makes these in-code references dead; a faithful move fixes what
it breaks:

- `xtask/src/steps/static_checks.rs` (wasm-clippy comment, ~L58-66) — says
  `web::pages` "compiles wasm-only… pulls `pages/` into the compile" →
  `web::app` / `app/component.rs`. The gate args (`-p web -p client -p csr`) are
  unchanged.
- `csr/index.html:4` (comment) — "byte-identical to
  `web::render::PREPAINT_SCRIPT`" → `web::app::PREPAINT_SCRIPT` (the drift-guard
  test keys on the value, so it still passes; only the prose name rots).
- `web/src/render/mod.rs` module header (L1-9) — currently describes the shell
  projector (`render_shell` "Shared by … `web::pages`") that is **leaving**;
  rewrite so the residual file no longer describes departed symbols nor names
  the deleted `web::pages`.
- The `DEFAULT_THEME` doc-comment (currently `render/mod.rs:20-23`, "re-exported
  from `pages` for the client") moves **with the const** to `app/render.rs` and
  must then read "re-exported from `app` (via `mod.rs`) for the client".

**Explicitly out of scope (pre-existing, not broken by this move):**
`docs/adr/0056-*` (superseded/frozen historical ADR); the `web::mount_csr()`
staleness on `ARCHITECTURE.md:17` (independent of `App`'s path).

## Doc updates (doc-parity — no doc may point at the deleted `web/src/pages/`)

Per the issue comment (deferred from #528) and the acceptance floor's
`docs/web-style-guide.md` doc-parity item:

- `CONTRIBUTING.md:24-25` (repo-layout) — drop `web/src/pages/`; describe the
  co-located vertical layout
  (`web/src/<vertical>/{mod,api,server,component}.rs`).
- `CONTRIBUTING.md:429-431` (`#[component]`-exemption paragraph) — the
  `web/src/pages/*` co-location example → the `component.rs` / vertical layout.
- `CONTRIBUTING.md:529-531` (coverage prose, "Leptos page components
  (`web/src/pages/*.rs`)") — point at `component.rs` files instead.
- `docs/README.md:17` — "Conventions for `web/src/pages/` components…" → drop
  the `pages/` path.
- `docs/web-style-guide.md:123-125` — the `web/src/pages/ui.rs` reference (that
  file is already gone via #657) → reflect the current co-located layout,
  without reopening the #312/#658 residual-render narrative.
- `docs/ARCHITECTURE.md:17` — "mounting `web::App`" → `web::app::App` (the
  crate-root `web::App` path is deleted). Leave the adjacent `web::mount_csr()`
  wording untouched (pre-existing staleness, separate concern).

## Non-goals

- **Not** deleting `web/src/render/mod.rs` — its residual half (`escape_html`,
  `Icons`, `TagCtx`, `render_hero`, `render_home_masthead`, `render_load_more`,
  `format_bytes` + their tests) is #658's job; whichever of #330/#658 lands last
  deletes the emptied file. #330 lands first here, so `render/mod.rs` stays.
- **No** `wasm-clippy` change — the struck-through simplification is superseded
  (#526): `-p web -p client` is permanent load-bearing gate surface.
- **No** behaviour change, renames, or API tightening beyond the file split.

## Acceptance criteria (observable)

1. `web/src/pages/` no longer exists;
   `rg 'web::pages|crate::pages|web/src/pages|mod pages|pages::'` over
   `web/ server/ csr/ xtask/ CONTRIBUTING.md docs/README.md docs/web-style-guide.md docs/ARCHITECTURE.md`
   returns nothing (archived `docs/archive/**`, `docs/superpowers/**`, and the
   frozen `docs/adr/0056-*` are excluded).
2. `web/src/app/` exists with `mod.rs`, `render.rs`, `component.rs`; the only
   `target_arch` gates in it are on `mod component;` and its re-export.
3. `web::app::App`, `web::app::render_shell`, `web::app::render_head`,
   `web::app::{DEFAULT_THEME,PREPAINT_SCRIPT,SPA_SHELL,DISCOVERY_MARKER_ATTR}`
   resolve; `web::App` and
   `web::render::{render_shell,render_head,SPA_SHELL, PREPAINT_SCRIPT,DISCOVERY_MARKER_ATTR,DEFAULT_THEME}`
   no longer resolve.
4. The pure shell projector's host tests (relocated) run and pass under
   `web::app::render` — including the `PREPAINT_SCRIPT`/`SPA_SHELL` drift guards
   and the `render_head`/`render_shell` coincidence tests.
5. `web::render` retains only the residual symbols + their tests;
   `pub mod render` still present in `lib.rs`.
6. No doc under `CONTRIBUTING.md`, `docs/README.md`, `docs/web-style-guide.md`,
   `docs/ARCHITECTURE.md` points at `web/src/pages/` or the deleted `web::App`
   path; no in-code comment (`xtask/src/steps/static_checks.rs`,
   `csr/index.html`, `web/src/render/mod.rs`) names `web::pages` or the departed
   shell symbols.
7. `#[cfg(target_arch = …)]` appears only on `mod` declarations repo-wide (#520
   `cargo xtask` check passes).
8. `cargo xtask validate` green: static + clippy (incl. wasm-clippy
   `-p web -p client -p csr`) + coverage + full e2e
   (`{sqlite,postgres}×{chromium,firefox}`). The projector-painted first paint
   and reactive boot still coincide (no flash) — exercised by the existing e2e
   flows.

## Verification

- Host: `cargo xtask check` (static + clippy + coverage) from the worktree.
- Wasm: `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
  before committing wasm-touching moves (`component.rs`).
- Full gate: `cargo xtask validate` (or `e2e-local` per combo) — the browser
  flows are the coincidence/no-flash regression surface.

# Plan — #330: delete `pages/`; rehome shell into `web::app`

**Spec:**
`docs/superpowers/specs/2026-07-24-issue-330-pages-delete-app-rehome.md` (the
"what/why"; this plan is the "how"). **For agentic workers:** drive with
**`jaunder-iterate`**, delegating a task via **`jaunder-dispatch`** if useful.

## Review header

**Goal.** Faithfully relocate the `pages/` shell (`App`/`AppShell`/Router) and
its pure render twin (`render_shell`/`render_head` + shell consts, currently in
`web::render`) into a new **`web::app`** vertical (ADR-0070 layout), then delete
`web/src/pages/`. Behaviour-identical.

**Scope — in:** the two-file `web::app` vertical + `mod.rs` wiring; stripping
the shell half out of `web::render` (residual half stays for #658); the Rust
consumer ripple (`web::render::X`/`web::App` → `web::app::X`); doc +
in-code-comment parity. **Scope — out:** deleting `render/mod.rs` (#658); any
`wasm-clippy` change (superseded); any behaviour change, rename, or API
tightening. No separable concerns surfaced → no issues to file.

**Tasks.**

1. Relocate the shell into `web::app` (atomic code move) + rewire all Rust
   consumers; `cargo xtask check` green.
2. Doc + in-code-comment parity (nothing points at `web/src/pages/` or
   `web::App`).
3. Full gate: `cargo xtask validate` (static + wasm-clippy + coverage + full
   e2e).

**Key risks / decisions.**

- **Atomic move, not split:** a pure relocation has no compiling half-way point
  (e.g. `pages` re-exports `DEFAULT_THEME` from what's being moved), so the code
  move is one commit. Done-ness is still checkable: `cargo xtask check` green +
  the 9 relocated host tests pass + `rg` shows `pages/` gone.
- **`web::app::render` is an ungated host leaf** (like `posts/render.rs`), so it
  keeps calling residual `crate::render::escape_html` /
  `crate::posts::render::…` cross-module — idiomatic, temporary until #658.
- **`target_arch` only on `mod` lines** (#520): the sole gates are
  `mod component;`
  - its `pub use` in `app/mod.rs`.
- **Coverage:** `app/render.rs` is host-compiled + measured (moved tests cover
  it); `app/component.rs` is wasm-only `#[component]`, structurally exempt. No
  new `cov:ignore`.

## Global constraints

- Rust; run everything from the worktree
  (`.claude/worktrees/issue-330-pages-delete-shell-rehome/`).
- Gate per task: `cargo xtask check` (host static + clippy + **wasm-clippy** +
  coverage). Before committing wasm-touching edits also run
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`.
- Commit via **`jaunder-commit`** (pre-commit runs full `cargo xtask check`).
  **No `Co-Authored-By` trailer.** Don't edit files mid-gate (Nix builds the
  working tree).
- Verbatim relocation: move function/const/test bodies unchanged; only paths and
  the residual-module doc header change.

---

## Task 1 — Relocate the shell into `web::app`; rewire consumers

Atomic code move. After it, `web/src/pages/` is gone, `web::app` exists, and the
whole workspace compiles.

### Files — create

- **`web/src/app/render.rs`** — the pure shell projector leaf. Move **verbatim**
  from `web/src/render/mod.rs`: `render_shell`, `render_head`,
  `render_discovery`, `feed_label`; consts `DEFAULT_THEME`, `PREPAINT_SCRIPT`,
  `SPA_SHELL`, `DISCOVERY_MARKER_ATTR` (carry each item's doc-comment; fix the
  `DEFAULT_THEME` doc "re-exported from `pages`" → "re-exported from `app` (via
  `mod.rs`)"). One moved test carries a stale assertion message —
  `index_html_shell_contains_the_prepaint_script` (`render/mod.rs:389`) asserts
  "…must embed `render::PREPAINT_SCRIPT` verbatim"; update that message to
  `app::PREPAINT_SCRIPT`. Add a module header modelled on `posts/render.rs`'s
  (ungated host leaf, ADR-0070; shared by `server::projector` + reactive
  `web::app`; coincidence per ADR-0041). Keep the cross-module calls:
  `crate::render::escape_html`, `crate::posts::render::render_body`,
  `crate::sidebar::render_sidebar`. Move the **9 shell tests** (per spec's Test
  partition) into this file's `#[cfg(test)] mod tests`, with
  `use crate::posts::render::test_fixtures::{one_post_page, sample_post};` and
  `use common::test_support::parse_username;`.
- **`web/src/app/component.rs`** — move **verbatim** from
  `web/src/pages/mod.rs`: `App` (pub `#[component]`), `AppShell` (private
  `#[component]`), `THEME_KEY`. Drop the top
  `pub use crate::render::DEFAULT_THEME;`; reference `DEFAULT_THEME` via
  `super::DEFAULT_THEME`. No `#[cfg]` inside the file.
- **`web/src/app/mod.rs`** — wiring only:

  ```rust
  //! The app shell vertical (#330, ADR-0070): `App` + the Router/route table and
  //! their pure projector twin. `render` is the host-compiled shell projector
  //! (shared with `server::projector`); `component` is the wasm-only reactive shell.
  mod render;
  pub use render::{
      render_head, render_shell, DEFAULT_THEME, DISCOVERY_MARKER_ATTR, PREPAINT_SCRIPT, SPA_SHELL,
  };

  #[cfg(target_arch = "wasm32")]
  mod component;
  #[cfg(target_arch = "wasm32")]
  pub use component::App;
  ```

### Files — edit

- **`web/src/lib.rs`** — remove `#[cfg(target_arch = "wasm32")] pub mod pages;`
  and `#[cfg(target_arch = "wasm32")] pub use pages::App;`. Add `pub mod app;`
  (place alphabetically, before `pub mod audiences;`). Also fix the
  `route_segments` doc-comment (L46-49): "…rather than under the wasm-only
  `pages` module that consumes it" → `app` (the segment is now consumed by
  `app::component`).
- **`web/src/render/mod.rs`** — delete the 8 moved items + the 9 moved tests.
  What remains: `escape_html`, `Icons`, `TagCtx`, `render_hero`,
  `render_home_masthead`, `render_load_more`, `format_bytes`, and the 6 residual
  tests (four `format_bytes_*`, `escape_replaces_markup_metacharacters`,
  `home_masthead_has_topbar_hero_and_anon_only_cta`). Rewrite the **module
  header (L1-9)** so it describes only the residual leaf primitives — no
  `render_shell`, no `web::pages`. Drop the imports the residual set no longer
  uses — `common::seed::PageSeed` and `std::fmt::Write`. **Keep
  `common::username::Username`** (still used by residual
  `TagCtx::ForUser(Username)`, `render/mod.rs:55`). Let clippy confirm the final
  import set.
- **`web/src/posts/render.rs:33`** — doc `[`crate::render::render_shell`]` →
  `[`crate::app::render_shell`]`.
- **`web/src/auth/marker.rs:5`** — doc `(`render::PREPAINT_SCRIPT`)` →
  `(`app::PREPAINT_SCRIPT`)`.
- **`server/src/projector/mod.rs`** — L44
  `use web::render::{render_head, render_shell, PREPAINT_SCRIPT};` →
  `use web::app::{render_head, render_shell, PREPAINT_SCRIPT};`; L392 test
  `web::render::PREPAINT_SCRIPT` → `web::app::PREPAINT_SCRIPT`.
- **`server/src/lib.rs`** — L120 `web::render::SPA_SHELL` →
  `web::app::SPA_SHELL`; L1 comment `web::App` → `web::app::App`.
- **`server/src/site.rs:141`** — `web::render::SPA_SHELL` →
  `web::app::SPA_SHELL`.
- **`csr/src/lib.rs`** — L8 `use web::App;` → `use web::app::App;`; L42
  `web::render::DISCOVERY_MARKER_ATTR` → `web::app::DISCOVERY_MARKER_ATTR`; L2
  comment `web::App` → `web::app::App`.

### Files — delete

- **`web/src/pages/mod.rs`** (and the now-empty `web/src/pages/` dir) via
  `git rm`.

### Verify

- `cargo xtask check` → **PASS** (host static + clippy + wasm-clippy
  `-p web -p client -p csr` + coverage). This compiles the wasm-gated
  `app/component.rs`, so a broken `App` move is caught here.
- The 9 relocated tests run under `web::app::render`:
  `cargo nextest run -p web app::render` → **PASS** (incl. the `PREPAINT_SCRIPT`
  / `SPA_SHELL` drift guards and `render_head`/`render_shell` coincidence
  tests).
- `rg 'web::pages|crate::pages|web/src/pages|mod pages|pages::' web/ server/ csr/`
  → nothing.
- `rg -i '\bpages\b' web/src/lib.rs web/src/app/` → nothing (catches the bare
  "`pages` module" prose the pattern above misses).
- `rg 'web::App|web::render::(render_shell|render_head|SPA_SHELL|PREPAINT_SCRIPT|DISCOVERY_MARKER_ATTR|DEFAULT_THEME)' web/ server/ csr/`
  → nothing.

### Commit

`jaunder-commit` — e.g.
`refactor(web): delete pages/, rehome App + shell projector into web::app (#330)`.

---

## Task 2 — Doc + in-code-comment parity

No code paths; nothing may point at the deleted `web/src/pages/` or `web::App`.

### Files — edit

- **`CONTRIBUTING.md`** — three refs (deferred from #528, per the issue
  comment): L24-25 repo-layout (`web/src/pages/` → the co-located
  `web/src/<vertical>/{mod,api,server,component}.rs` layout); L429-431 the
  `#[component]`-exemption paragraph's `web/src/pages/*` example →
  `component.rs` / vertical layout; L529-531 coverage prose "Leptos page
  components (`web/src/pages/*.rs`)" → the `component.rs` files.
- **`docs/README.md:17`** — "Conventions for `web/src/pages/` components and
  widgets" → drop the `pages/` path (e.g. "…for the `web/` Leptos components and
  widgets").
- **`docs/web-style-guide.md:123-125`** — the `web/src/pages/ui.rs` reference
  (file already deleted by #657) → reflect the current co-located layout; do
  **not** reopen the #312/#658 residual-render narrative.
- **`docs/ARCHITECTURE.md:17`** — "mounting `web::App`" → `web::app::App`; leave
  the adjacent `web::mount_csr()` wording (pre-existing, out of scope).
- **`xtask/src/steps/static_checks.rs`** (wasm-clippy comment, ~L58-66) —
  `web::pages` / "pulls `pages/` into the compile" → `web::app` /
  `app/component.rs`. Comment only; gate args unchanged.
- **`csr/index.html:4`** (comment) — "byte-identical to
  `web::render::PREPAINT_SCRIPT`" → `web::app::PREPAINT_SCRIPT`.

### Verify

- `rg 'web/src/pages|web::pages|web::App' CONTRIBUTING.md docs/README.md docs/web-style-guide.md docs/ARCHITECTURE.md xtask/ csr/index.html`
  → nothing (archived `docs/archive/**`, `docs/superpowers/**`, frozen
  `docs/adr/0056-*` excluded).
- `cargo xtask check` → **PASS** (the `static_checks.rs` edit recompiles xtask;
  the `csr/index.html` `PREPAINT_SCRIPT` drift-guard test still passes — it keys
  on the value, not the comment).
- Pre-commit prettier leaves the Markdown clean (re-stage if it reflows).

### Commit

`jaunder-commit` — e.g.
`docs(web): repoint pages/ + web::App references at web::app (#330)`.

---

## Task 3 — Full local gate

- `cargo xtask validate` (foreground, `timeout: 600000`; or `e2e-local <spec>`
  per combo) → **PASS**: static + wasm-clippy + coverage + full e2e
  (`{sqlite,postgres}×{chromium,firefox}`). The projector first paint ↔
  reactive boot coincidence (no flash) is exercised by the existing
  home/timeline/permalink e2e flows — the regression surface for this move.
- If green, the branch is ready for **`jaunder-ship`** (final review → PR →
  merge HALT).

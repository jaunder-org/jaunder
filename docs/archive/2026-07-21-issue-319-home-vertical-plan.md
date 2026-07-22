# Home Vertical Convergence + ADR-0041 Coincidence (#319) Implementation Plan

> **For agentic workers:** Execute task-by-task with **jaunder-iterate**
> (delegating a task to a subagent via **jaunder-dispatch** when useful). Steps
> use checkbox (`- [ ]`) syntax.

**Spec:** `docs/superpowers/specs/2026-07-21-issue-319-home-vertical.md` — the
"what/why." This plan is the "how."

**Goal:** Converge the `home` vertical onto the co-located layout **and**
restore ADR-0041 coincidence: the reactive `HomePage` renders its masthead
(topbar + Sign-in/Register + hero) via `inner_html` of one shared pure fn — the
same fn the projector calls — instead of parallel `view!` markup; and fix the
authed-owner Sign-in/Register flash by giving that fn `j-anon-only` links.

**Architecture:** A pure `render_home_masthead()` in `web/src/render/mod.rs` is
the single source for both the host projector (`posts::render::render_body`) and
the wasm `HomePage` (via `<div style="display:contents" inner_html=…>`, the
established `Sidebar` pattern). `home` becomes a two-file vertical
(`mod.rs`/`component.rs`).

**Tech Stack:** Rust, Leptos (CSR/wasm), Playwright e2e, `cargo xtask`.

## Review header

**Scope — in:** the shared masthead fn + `render_body` repoint + `j-anon-only`
flash fix (host); rewriting `HomePage` onto the `inner_html` masthead
(reactive); the `pages/home.rs` → `home/` move; a unit-test update + an
owner-view e2e assertion. **Scope — out:** moving pure fns into verticals /
`markup.rs` / dissolving `web::render` (**#312**); making reactive `<Topbar>`
itself `inner_html`-backed; `read_signal!` inlining (**#304**); cockpit
(**#317**); App/Router (**#330**).

**Tasks:**

1. Projector: `render_home_masthead()` + repoint `render_body` + `j-anon-only`
   flash fix + host tests + owner-view e2e assertion.
2. Reactive: rewrite `HomePage` (in `pages/`) onto the `inner_html` masthead.
3. Move `pages/home.rs` → `home/` vertical; `lib.rs` + router repoint; delete.

**Key risks/decisions:**

- **Coincidence is the invariant.** Both projector and reactive call the _one_
  `render_home_masthead()`, so they cannot drift (ADR-0041 §2).
  `render_hero()`'s "mirroring home.rs" twin is eliminated.
- **`render_home_masthead` stays live on wasm** transitively via
  `pub fn render_shell` → `render_body` → it, so Task 1 gates clean before the
  reactive caller lands (Task 2).
- **Flash fix mechanism** (verified): `html.authed .j-anon-only{display:none}`
  (`jaunder.css:1287`, specificity beats `.j-btn`) + the pre-paint script set
  `html.authed` before paint → owner's server-painted CTA hidden; anon (`<html>`
  without `.authed`) still sees it.
- **`HomePage` drops the reactive `<Topbar>` + `crate::pages::ui` import** — the
  topbar now comes from the pure `topbar::render` inside the masthead.

## Global Constraints

- No `Co-Authored-By` trailer on commits.
- `target_arch = "wasm32"` only on `mod`-wiring lines (ADR-0070 §2);
  `component.rs` has zero internal cfgs.
- Pure render fns stay in `web/src/render` (ADR-0041 §1); no `markup.rs` in a
  vertical, no `auth` change (that is #312).
- Per-commit gate: `cargo xtask check` (fmt + clippy + **wasm-clippy** + Nix
  coverage/tests). Run it first so the hook passes clean (**jaunder-commit**).
  Local e2e is reaped — `validate --no-e2e` locally; CI's matrix gates e2e.

---

### Task 1: Projector — shared masthead fn + flash fix (host TDD)

**Files:**

- Modify: `web/src/render/mod.rs` (add `render_home_masthead` after
  `render_hero`, ~`:237`; add a host test)
- Modify: `web/src/posts/render.rs` (`render_body` `SiteTimeline` arm `:44-64`;
  the `use crate::render::{…}` at `:17`; the unit test `:454-465`)
- Modify: `end2end/tests/authed-flash.spec.ts` (owner test, after `:32`)

**Interfaces:**

- Produces: `pub(crate) fn render_home_masthead() -> String` (in
  `crate::render`), consumed by `render_body` (this task) and `HomePage` (Task
  2).

- [x] **Step 1: Write the failing host tests**

In `web/src/render/mod.rs`'s `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn home_masthead_has_topbar_hero_and_anon_only_cta() {
    let html = render_home_masthead();
    assert!(html.contains("<h1>jaunder.local</h1>"), "{html}");
    assert!(
        html.contains("<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>"),
        "{html}"
    );
    assert!(
        html.contains("<a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>"),
        "{html}"
    );
    assert!(html.contains("<div class=\"j-hero\">"), "{html}");
}
```

Update the existing `home_local_body_has_topbar_hero_signin_and_posts`
(`posts/render.rs:457-464`) — the CTA now carries `j-anon-only`:

```rust
    assert!(
        html.contains("<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>"),
        "{html}"
    );
    assert!(
        html.contains("<a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>"),
        "{html}"
    );
```

- [x] **Step 2: Run, verify they fail**

Run: `cargo nextest run -p web render::` Expected: FAIL — `render_home_masthead`
undefined; the `render_body` assertion fails (still emits `class="j-btn"`).

- [x] **Step 3: Implement `render_home_masthead` + repoint `render_body`**

Add to `web/src/render/mod.rs` (after `render_hero`):

```rust
/// The home page masthead — the topbar (with the anonymous Sign-in / Register
/// links) then the hero. The single source both the projector
/// (`crate::posts::render::render_body`) and the reactive `home::HomePage` render,
/// so coincidence holds by construction (ADR-0041 §2) — no `view!` twin to drift.
/// The links carry `j-anon-only` so the authed owner's pre-painted masthead hides
/// them (ADR-0044); an anonymous viewer (no `html.authed`) still sees them.
#[must_use]
pub(crate) fn render_home_masthead() -> String {
    format!(
        "{topbar}{hero}",
        topbar = crate::topbar::render(
            "jaunder.local",
            Some("Read-only \u{00b7} posts originating on this instance"),
            "<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>\
             <a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>",
        ),
        hero = render_hero(),
    )
}
```

In `web/src/posts/render.rs`, rewrite the `SiteTimeline` arm (`:44-64`) to:

```rust
        // Home (anonymous "Local" mode): the shared masthead + a bare `j-scroll`.
        PageSeed::SiteTimeline(page) => {
            let scroll = if page.posts.is_empty() {
                "<p>No posts yet.</p>".to_string()
            } else {
                format!(
                    "{}{}",
                    render_posts(&page.posts, &TagCtx::SiteWide),
                    render_load_more(page.has_more),
                )
            };
            format!(
                "{masthead}<div class=\"j-scroll\">{scroll}</div>",
                masthead = crate::render::render_home_masthead(),
            )
        }
```

Drop `render_hero` from the `use crate::render::{…}` block at
`posts/render.rs:17` (`render_body` no longer calls it directly;
`render_home_masthead` does). Leave
`escape_html, render_load_more, PageSeed, TagCtx` and the `topbar` import (still
used by the Profile/Tag arms).

Also fix `render_hero`'s now-stale doc (`render/mod.rs:229-230`) — it no longer
"mirrors `home.rs`"; it is composed into `render_home_masthead`, the single
source both sides render:

```rust
/// The home page hero block (constant copy). Composed into
/// [`render_home_masthead`] — the one source the projector and the reactive
/// `home::HomePage` both render (ADR-0041 §2), so there is no `view!` twin.
```

- [x] **Step 4: Add the owner-view e2e assertion**

In `end2end/tests/authed-flash.spec.ts`, in the owner `/` test, after the
`topbarHeading` assertion (`:32`), add:

```ts
// #319: the anon Sign-in/Register CTA is server-painted but `j-anon-only`, so
// the pre-paint `html.authed` hides it for the owner (no flash). Use CSS
// locators (which match hidden nodes) so this asserts present-but-hidden, not
// merely absent — `getByRole` skips `display:none` elements and would pass
// vacuously.
await expect(page.locator('main a[href="/login"]')).toBeHidden();
await expect(page.locator('main a[href="/register"]')).toBeHidden();
```

- [x] **Step 5: Run tests + gate**

Run: `cargo nextest run -p web render::` Expected: PASS (masthead + updated
`render_body` tests). Run: `cargo xtask check` Expected: green (host clippy,
wasm-clippy — `render_home_masthead` is live on wasm via `pub fn render_shell`;
coverage/tests). No `cov:ignore` — the fn is pure and host-tested.

- [x] **Step 6: Commit**

```bash
git add web/src/render/mod.rs web/src/posts/render.rs end2end/tests/authed-flash.spec.ts
git commit -m "fix(web/render): single home-masthead fn + hide anon CTA from owner

Extract render_home_masthead() as the one source both the projector
(render_body) and (next commit) the reactive HomePage render, restoring
ADR-0041 coincidence. Give the Sign-in/Register links j-anon-only so the
authed owner's server-painted masthead hides them pre-paint (ADR-0044), with
an owner-view e2e assertion. No visible change for anonymous visitors."
```

Run `cargo xtask check` first so the hook passes clean.

---

### Task 2: Reactive — render the masthead via `inner_html` (in place)

Rewrite `HomePage`'s masthead onto the shared fn. Still in `pages/home.rs` (the
move is Task 3), so the diff is purely the coincidence fix.

**Files:**

- Modify: `web/src/pages/home.rs` (imports `:1-9`; the `view!` `:58-93`)

**Interfaces:**

- Consumes: `crate::render::render_home_masthead` (Task 1).

- [x] **Step 1: Rewrite the masthead render**

Drop `use crate::pages::ui::Topbar;` (`home.rs:5`). Replace the `view!` block
(`:58-93`) with:

```rust
    let read_error = Memo::new(move |_| read_signal!(state.status).into_failure());
    let masthead = crate::render::render_home_masthead();

    view! {
        <FeedDiscovery surface=FeedSurface::Site />
        {move || {
            if let Some(err) = read_error.get() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            view! {
                <div style="display:contents" inner_html=masthead.clone()></div>
                <TimelineRows state=state on_mutate=on_mutate on_load_more=on_load_more />
            }
                .into_any()
        }}
    }
```

(The `read_error`/`masthead` `let`s replace the old `read_error` line; keep the
seed-adopt, `server_resource`, `Effect`, and `on_*` callbacks above unchanged.)

- [x] **Step 2: Gate**

Run: `cargo xtask check` Expected: green — **`wasm-clippy`** is the load-bearing
check (the masthead render and dropped `<Topbar>`/`pages::ui` import compile on
wasm). No unused-import warning (`Topbar` gone; all remaining imports still
used).

- [x] **Step 3: Verify coincidence didn't regress (host)**

Run: `cargo nextest run -p web render::` Expected: PASS — the projector bytes
are unchanged from Task 1; the reactive side now calls the same fn.

- [x] **Step 4: Commit**

```bash
git add web/src/pages/home.rs
git commit -m "refactor(web/home): render the masthead via the shared pure fn

HomePage renders the topbar+hero+CTA via inner_html=render_home_masthead()
(the Sidebar pattern) instead of parallel view! markup, so it coincides with
the projector by construction (ADR-0041 §2). Drops the reactive <Topbar> and
the crate::pages::ui import. No DOM change."
```

---

### Task 3: Move `home` into its co-located vertical

Pure relocation of the (now coincidence-correct) `HomePage`.

**Files:**

- Create: `web/src/home/component.rs`, `web/src/home/mod.rs`
- Modify: `web/src/lib.rs` (add `pub mod home;`, alpha order: after
  `pub mod forms;`, before `pub mod icon;`)
- Modify: `web/src/pages/mod.rs` (`:3` remove `pub mod home;`; `:28` repoint the
  import)
- Delete: `web/src/pages/home.rs`

- [x] **Step 1: Create the vertical**

`git mv web/src/pages/home.rs web/src/home/component.rs`, then prepend a `//!`
doc to `component.rs`:

```rust
//! The home vertical's wasm-only UI (ADR-0070): the routed `/` public
//! Local-timeline landing page. Renders the shared `crate::render` masthead via
//! `inner_html` (coincidence with the projector, ADR-0041) + the reactive
//! `crate::timeline` rows. No cfgs of its own (wasm-only via its `mod` line).
```

Create `web/src/home/mod.rs`:

```rust
//! The home vertical (#319, ADR-0070): the routed `/` public Local-timeline
//! landing page. Module wiring only — a server-less, logic-free vertical (no
//! `api.rs`/`server.rs`/`state.rs`); its `component` composes `crate::timeline`
//! and the shared `crate::render` masthead.

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::HomePage;
```

- [x] **Step 2: Wire it up + repoint the router**

Add to `web/src/lib.rs` (after `pub mod forms;`):

```rust
pub mod home;
```

In `web/src/pages/mod.rs`: delete `pub mod home;` (`:3`); change
`use crate::pages::home::HomePage;` (`:28`) to `use crate::home::HomePage;`.

- [x] **Step 3: Verify no stale references**

Run: `rg -n 'pages::home' web/src` Expected: no hits (the module + router import
are repointed). Run: `rg -n 'pages::ui::Topbar|<Topbar' web/src/home/` Expected:
no hits (home uses neither the `pages::ui` Topbar shim nor the reactive
`<Topbar>`). **Do not grep the whole tree** for `pages::ui::Topbar` —
`cockpit.rs` legitimately keeps it (that is #317, out of scope).

- [x] **Step 4: Gate**

Run: `cargo xtask check` Expected: green (wasm-clippy resolves
`crate::home::HomePage`; the router mounts it; host build sees an empty `home`
module).

- [x] **Step 5: Commit**

```bash
git add web/src/home/component.rs web/src/home/mod.rs web/src/lib.rs web/src/pages/mod.rs
git commit -m "refactor(web/home): relocate HomePage into the home vertical

Move pages/home.rs -> web/src/home/{mod.rs,component.rs} (ADR-0070 §5), a
server-less two-file vertical; repoint lib.rs + the router import; delete
pages/home.rs. No behaviour change."
```

(The `git mv` deletion of `pages/home.rs` is already staged.)

- [x] **Step 6: Full-gate verification**

Run: `cargo xtask validate --no-e2e` Expected: green. Local e2e is reaped — CI's
`{sqlite,postgres}×{chromium,firefox}` matrix gates the `/` flows and the new
owner-view assertion. Confirm a clean tree (`git status --porcelain`).

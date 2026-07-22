# Cockpit vertical convergence (#317) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with **jaunder-iterate**
> (delegating to a subagent via **jaunder-dispatch** when useful). Steps use checkbox
> (`- [ ]`) syntax for tracking.

**Goal:** Relocate `CockpitPage` from the legacy `web/src/pages/cockpit.rs` into a
co-located, server-less ADR-0070 vertical `web/src/cockpit/`, fully decoupled from the
`pages/` module.

**Architecture:** Server-less two-file vertical mirroring home #319 — `mod.rs` (wiring
only) + wasm-only `component.rs`; no `api.rs`/`server.rs`. The component moves verbatim
except for two `pages/`-decoupling edits (inline `read_signal!` → `.get()`; import
`Topbar` from `crate::topbar`). `pages/cockpit.rs` is already wasm-only, so no gating
change.

**Tech Stack:** Rust, Leptos CSR, `cargo xtask` gate, Playwright (`end2end/`).

**Spec:** `docs/superpowers/specs/2026-07-22-issue-317-cockpit-vertical.md` (Decisions
1–6, AC1–AC6).

## Global Constraints

- **Single atomic move** — the relocation must land as one compiling commit (a
  half-moved state won't build).
- **Behavior-preserving** — no change to `CockpitPage`'s logic, the `/app` route, or
  the `current_user` gate / `/login` bounce.
- **No new `cov:ignore` / `crap:allow` markers.** `component.rs` is wasm-only (not
  host-coverage-measured), as `pages/cockpit.rs` already was.
- **Gate:** pre-commit hook runs full `cargo xtask check`; run it before committing
  (**jaunder-commit**). **No `Co-Authored-By` trailer.**
- **Local e2e is reaped here** — gate locally with `cargo xtask check` /
  `validate --no-e2e`; cockpit's Playwright flows (`authed-flash.spec.ts`,
  `/app` flows in `posts.spec.ts`/`media.spec.ts`) are CI-verified on the PR.

---

## Task list (one line each)

1. Relocate `CockpitPage` into `web/src/cockpit/` (server-less vertical) and decouple
   it from `pages/`.

**Key risks/decisions:** must be one atomic commit; `pages/mod.rs` is wasm-only so its
`use crate::cockpit::CockpitPage` is ungated (like `crate::home::HomePage`); `Topbar`
from `crate::topbar` is provably equivalent (`pages::ui::Topbar` is just a re-export of
`topbar::Topbar`); `pages::signal_read`/`pages::ui` stay (other `pages/` files use them).

---

### Task 1: Relocate `CockpitPage` into the `cockpit` vertical, decoupled from `pages/`

**Files:**

- Create: `web/src/cockpit/mod.rs`
- Create: `web/src/cockpit/component.rs`
- Modify: `web/src/lib.rs` (add `pub mod cockpit;` after `pub mod backup;`, `:24`)
- Modify: `web/src/pages/mod.rs` (drop `pub mod cockpit;` `:1`; change import `:26`)
- Delete: `web/src/pages/cockpit.rs`

**Interfaces:**

- Consumes: nothing new.
- Produces: `crate::cockpit::CockpitPage` (wasm-only), replacing
  `crate::pages::cockpit::CockpitPage`. The `/app` route (`pages/mod.rs:114`) resolves
  to it unchanged.

_No host unit test exists (wasm-only component); verification is the gate (compile +
wasm-clippy + coverage) and the CI e2e (AC5). The file contents are the contract._

- [ ] **Step 1: Create `web/src/cockpit/mod.rs`** (wiring only, mirroring
  `home/mod.rs`):

```rust
//! The cockpit vertical (#317, ADR-0070): the routed `/app` authed-only
//! personalized Feed (#181, ADR-0044 D6). Module wiring only — a server-less
//! vertical (no `api.rs`/`server.rs`); its `component` composes `crate::auth`,
//! `crate::posts`, `crate::timeline`, and the shared `crate::topbar`.

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::CockpitPage;
```

- [ ] **Step 2: Create `web/src/cockpit/component.rs`** — the exact current body of
  `web/src/pages/cockpit.rs` (its module doc comment lines 1-6 → `component.rs`'s `//!`
  header; the `#[component] pub fn CockpitPage` unchanged), with these **three edits**:
  - **Drop** `use crate::pages::signal_read::read_signal;` and
    `use crate::pages::ui::Topbar;`.
  - **Add** `use crate::topbar::Topbar;` (keep the other imports: `common::pagination::PageSize`,
    `common::username::Username`, `leptos::prelude::*`, `leptos_router::components::Redirect`,
    `crate::auth::current_user`, `crate::posts::{list_home_feed, InlineComposer}`,
    `crate::timeline::{TimelineRows, TimelineState}`).
  - **Inline** the three `read_signal!` calls (the macro is exactly `$signal.get()`):
    - `let read_error = Memo::new(move |_| read_signal!(state.status).into_failure());`
      → `... state.status.get().into_failure());`
    - `let read_bounce = move || read_signal!(bounce);` → `... move || bounce.get();`
    - `let read_username = move || read_signal!(username);` → `... move || username.get();`

  Everything else (the `current_user` gate, the `bounce`/`username` anti-remount guards,
  the `Effect`, `on_load_more`, the `view!` with `Topbar`/`InlineComposer`/`TimelineRows`)
  is copied **verbatim**.

- [ ] **Step 3: Register the vertical at the web crate root.** In `web/src/lib.rs`, add
  `pub mod cockpit;` in alphabetical order — immediately after `pub mod backup;`
  (`lib.rs:24`), before `pub mod email;`.

- [ ] **Step 4: Rewire `web/src/pages/mod.rs`.**
  - Delete `pub mod cockpit;` (line 1).
  - Change `use crate::pages::cockpit::CockpitPage;` (line 26) →
    `use crate::cockpit::CockpitPage;` (ungated, matching `use crate::home::HomePage;`
    at line 24). Move it to the crate-level import group if fmt requires (rustfmt will
    reorder).
  - Leave `pub(crate) mod signal_read;`, `pub mod ui;`, and the `/app` route
    (`view=CockpitPage`, line 114) **unchanged**.

- [ ] **Step 5: Delete `web/src/pages/cockpit.rs`.**

```bash
git rm web/src/pages/cockpit.rs
```

- [ ] **Step 6: Run the gate, verify clean.**

Run: `cargo xtask check --no-test`
Expected: PASS — compiles (host + wasm), clippy + wasm-clippy clean, no unresolved
`crate::pages::cockpit` / `CockpitPage` reference, no dead `read_signal`/`Topbar`
import left behind.

- [ ] **Step 7: Confirm the decoupling and the move (AC1–AC4).**

Run: `rg -n 'crate::pages::' web/src/cockpit/component.rs` → Expected: no hits (AC3).
Run: `rg -rn 'pages::cockpit|pages/cockpit' web/src` → Expected: no hits.
Run: `rg -n 'pub mod cockpit' web/src/lib.rs web/src/pages/mod.rs` → Expected: only
`lib.rs` (AC4).

- [ ] **Step 8: Commit.**

```bash
git add web/src/cockpit/mod.rs web/src/cockpit/component.rs web/src/lib.rs web/src/pages/mod.rs
git commit -m "refactor(web/cockpit): co-locate CockpitPage into its own vertical, decouple from pages/"
```

Run `cargo xtask check` first so the pre-commit gate passes clean (**jaunder-commit**).
The deletion of `pages/cockpit.rs` (git rm, Step 5) is included in the commit.

---

## Self-review

- **Spec coverage:** AC1 (Steps 1-2, 5 — files created, old deleted); AC2 (Step 1 —
  wiring-only mod.rs); AC3 (Step 2 edits + Step 7 `rg`); AC4 (Steps 3-4 + Step 7 `rg`);
  AC5 (behavior verbatim, CI e2e); AC6 (Steps 6, 8 gate). All map.
- **No separable concerns** — the whole change is one atomic relocation; `read_signal!`'s
  wider removal (#304) and `pages/` deletion (#330) are explicitly out of scope.
- **Type/name consistency:** `CockpitPage` name unchanged; `Topbar` resolves from
  `crate::topbar` (verified re-export chain); `read_signal!(x)` → `x.get()` is the exact
  macro expansion.
- **Coverage note:** all moved code is wasm-only (`component.rs`), as `pages/cockpit.rs`
  already was — no host-coverage delta, no new markers expected.

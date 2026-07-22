# #516 — client navigation + confirm-dialog primitives Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating via **jaunder-dispatch** when useful). Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `web`'s last raw `window().location()` / `confirm_with_message`
glue into two wasm-only `client` modules (`navigation`, `dialog`).

**Architecture:** Spec →
[`docs/superpowers/specs/2026-07-22-issue-516-nav-confirm-primitives.md`](../specs/2026-07-22-issue-516-nav-confirm-primitives.md).
Task 1 adds the primitives (raw `web-sys` wrappers, unconditional modules like
`client::storage`); Task 2 rewrites the 7 `web` call sites to them, dropping the
`web_sys::window()` guards. No host logic — validated by the csr wasm build +
the existing posts e2e.

**Tech Stack:** Rust, `web-sys`, wasm32, `cargo xtask` gate.

## Global Constraints

- **`client` charter (ADR-0069):** raw browser glue only, **no domain types**.
  `navigation`/`dialog` depend only on `web-sys` (no `leptos`) →
  **unconditional** modules, no `csr` feature gate.
- **Behaviour-preserving:** each primitive matches the code it replaces —
  swallowed navigation `Result` (`let _ =`), `confirm` `unwrap_or(false)`,
  off-DOM no-op.
- **Gate:** `cargo xtask check` (pre-commit hook, full gate) clean before each
  commit (**jaunder-commit**). **No `Co-Authored-By` trailer.**

---

## Review header

**Goal:** As above — two `client` primitives, the last `location`/`confirm` glue
out of `web`.

**Scope — in:** `client::navigation::{replace,reload}`,
`client::dialog::confirm`; `web-sys` `"Location"` feature; rewrite 4 `replace` +
1 reload + 2 `confirm` sites in `posts/component.rs` and `pages/mod.rs`.

**Scope — out:** `media/component.rs` fetch glue (#517); `lib.rs` DOM-seed reads
(#519); `js_sys::Date` (#518); the endgame ratchet (#520). No typed-URL work
beyond `impl AsRef<str>`.

**Tasks:**

1. Add `client::navigation` + `client::dialog` modules + wire the `web-sys`
   `"Location"` feature and the `lib.rs` module declarations.
2. Sweep the 7 `web` call sites to the primitives; drop the `web_sys::window()`
   guards and any now-unused `web_sys` imports.

**Key risks/decisions:**

- **Wasm-only, not host-compiled:** both the primitives and the rewritten call
  sites are validated by `cargo build -p csr --target wasm32-unknown-unknown`
  (inside `cargo xtask check`), not host `cargo test`. A green host build proves
  nothing.
- **`reload()` semantics:** keep the `href()` + `replace(href)` pattern (a
  history-replacing re-request), **not** `location().reload()` —
  behaviour-preserving with `posts/component.rs:1212-1214`.

---

### Task 1: Add `client::navigation` + `client::dialog` + wiring

**Files:**

- Create: `client/src/navigation.rs`, `client/src/dialog.rs`.
- Modify: `client/src/lib.rs` — add `pub mod navigation;` + `pub mod dialog;`
  (after `pub mod storage;`, unconditional).
- Modify: `client/Cargo.toml` — add `"Location"` to the `web-sys` `features`
  list.

**Interfaces:**

- Produces (consumed by Task 2 and future verticals):

```rust
// client::navigation
pub fn replace(url: impl AsRef<str>);
pub fn reload();
// client::dialog
pub fn confirm(message: &str) -> bool;
```

- [x] **Step 1: `client/src/navigation.rs`** — the two fns to the exact bodies
      in the spec (Decision §):

```rust
//! Raw browser navigation primitives (`window.location`). Wasm-only; no domain types.

/// Replace the current history entry with `url` (`location.replace`). No-op off-DOM;
/// the navigation `Result` is swallowed, matching the `web` call sites it replaces.
pub fn replace(url: impl AsRef<str>) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().replace(url.as_ref());
    }
}

/// Reload the current URL by replacing it with itself — a full server round-trip
/// (hands a non-SPA route back to the server). No-op off-DOM.
pub fn reload() {
    if let Some(window) = web_sys::window() {
        let location = window.location();
        if let Ok(href) = location.href() {
            let _ = location.replace(&href);
        }
    }
}
```

- [x] **Step 2: `client/src/dialog.rs`**:

```rust
//! Raw browser dialog primitives (`window.confirm`). Wasm-only; no domain types.

/// Show a native confirm dialog; `true` only if the user confirmed. `false` off-DOM
/// or if the dialog can't be shown (matching the current `unwrap_or(false)`).
pub fn confirm(message: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(message).ok())
        .unwrap_or(false)
}
```

- [x] **Step 3: Wire `lib.rs` + Cargo.** In `client/src/lib.rs` add, after
      `pub mod storage;`:

```rust
pub mod navigation;
pub mod dialog;
```

In `client/Cargo.toml`, extend the `web-sys` features to
`["Window", "Storage", "Location"]`.

- [x] **Step 4: Verify the wasm build compiles the modules.**

  Run: `cargo check -p client --features csr --target wasm32-unknown-unknown`
  (or without `--features csr` — these modules are unconditional; the csr build
  also covers them) → Expected: **PASS**, no warnings. Run: `cargo xtask check`
  → Expected: **PASS** (host: `client` stays empty-on-host, so no leptos/domain
  dep is pulled; the csr wasm build compiles `navigation`/`dialog`).

- [x] **Step 5: Commit.**

```bash
git add client/src/navigation.rs client/src/dialog.rs client/src/lib.rs client/Cargo.toml Cargo.lock
git commit -m "feat(client): add navigation + dialog browser primitives (#516)"
```

---

### Task 2: Sweep the `web` call sites to the primitives

**Files:**

- Modify: `web/src/posts/component.rs` — the 4 `replace` (`:239`, `:1231`,
  `:1581`)
  - 1 reload (`:1212-1214`) + 2 `confirm` (`:254`, `:296`) sites, each currently
    wrapped in an `if let Some(window) = web_sys::window()` guard (lines `238`,
    `1212`, `1230`, `1580`) or the `web_sys::window().and_then(...)` confirm
    chain (`253`, `295`).
- Modify: `web/src/pages/mod.rs` — the `replace(loc)` site at `:79-81`.

**Interfaces:**

- Consumes: `client::navigation::{replace, reload}`, `client::dialog::confirm`
  (Task 1).

- [x] **Step 1: Rewrite the sites** (drop the `web_sys::window()` guards):
  - `posts/component.rs:238-240` →
    `client::navigation::replace(&published.permalink);`
  - `posts/component.rs:253-255` →
    `let confirmed = client::dialog::confirm("Publish this draft?");`
  - `posts/component.rs:295-297` →
    `let confirmed = client::dialog::confirm("Delete this post?");`
  - `posts/component.rs:1212-1216` → `client::navigation::reload();`
  - `posts/component.rs:1230-1232` → `client::navigation::replace("/drafts");`
  - `posts/component.rs:1580-1582` →
    `client::navigation::replace(&updated.permalink);`
  - `pages/mod.rs:79-81` → `client::navigation::replace(loc);`

  (`&published.permalink` / `&updated.permalink` are `RootRelativeUrl`, accepted
  via `impl AsRef<str>`.)

- [x] **Step 2: Drop now-unused imports.** These call sites are fully-qualified
      `web_sys::window()` (no `use web_sys` import in either file), so there is
      likely nothing to remove — just confirm `cargo xtask check`'s clippy stays
      clean (it flags any `unused_imports`). After the sweep,
      `rg 'web_sys' web/src/posts/component.rs web/src/pages/mod.rs` should
      return nothing.

- [x] **Step 3: Verify.**

  Run: `rg '\.location\(\)|confirm_with_message' web/src` → Expected: **no
  matches** (AC#2). Use `\.location\(\)`, not `location\(\)` — the latter
  false-matches leptos's `use_location()` router hook at `pages/ui.rs:77`, which
  is out of scope. Run: `cargo build -p csr --target wasm32-unknown-unknown` →
  Expected: **PASS**. Run: `cargo xtask check` then
  `cargo xtask validate --no-e2e` → Expected: **PASS** (AC#7). Posts
  publish/unpublish/delete redirect + confirm flows run in CI e2e.

- [x] **Step 4: Commit.**

```bash
git add web/src/posts/component.rs web/src/pages/mod.rs
git commit -m "refactor(web): use client navigation/dialog primitives; drop direct window calls (#516)"
```

---

## Self-review (done at write time)

- **Spec coverage:** AC1→T1 (primitives); AC2/AC3→T2 (sweep, rg-empty); AC4→T1
  (feature + unconditional mods); AC5→(both, out-of-scope files untouched);
  AC6→T2 (behaviour-preserving rewrites + CI e2e); AC7→T2 (gate). All mapped.
- **Placeholders:** none — full bodies + exact commands.
- **Type consistency:** `replace(impl AsRef<str>)` accepts `&RootRelativeUrl`,
  `&str`, and `String`; `confirm(&str) -> bool` matches the call sites; module
  paths `client::navigation` / `client::dialog` are stable across tasks.

# Plan — Move the CSR boot out of `web` (#519)

Spec:
[`2026-07-22-issue-519-csr-boot-out-of-web.md`](../specs/2026-07-22-issue-519-csr-boot-out-of-web.md).
The spec is the "what/why" (the three-way split + ACs); this plan is the "how."
Don't re-derive the analysis — see the spec.

## Review header

- **Goal:** Relocate the CSR boot (`mount_csr` + `read_dom_seed`) out of
  `web/src/lib.rs` — generic DOM primitives to `client`, the boot to a
  now-wasm-only `csr`, the typed seed parse inline against the existing
  `web::render::PageSeed` — so `web/src/lib.rs` carries `target_arch` only on
  mod-decl/re-export lines with no `feature = "csr"` item gate and no
  `cov:ignore`.
- **Scope (in):** `client/src/dom.rs` (new) + `client/src/lib.rs` +
  `client/Cargo.toml`; `csr/src/lib.rs` + `csr/Cargo.toml`; `web/src/lib.rs`
  (strip the boot); `xtask/src/steps/static_checks.rs` + `flake.nix` (extend
  `wasm-clippy` to `-p csr`).
- **Scope (out):** `web::render` is **not** touched (`PageSeed` stays put; its
  move to `common` is #610); `pub use pages::App` wasm gate untouched (App moves
  with #330); no new e2e (existing matrix covers both boot paths).
- **Tasks:**
  1. Add the `client::dom` primitives (+ web-sys features).
  2. Extend `wasm-clippy` to lint `-p csr` (xtask step + unit test + flake
     derivation).
  3. Move the boot into `csr`, make `csr` wasm-only, strip `web/src/lib.rs`.
  4. Full gate.
- **Key risks/decisions:**
  - **Atomicity (task 3):** removing `mount_csr` from `web` breaks `csr`'s
    `web::mount_csr()` call, so the `web/src/lib.rs` strip and the `csr` boot
    land in **one commit**.
  - **Ordering:** task 1 before task 3 (`csr` calls `client::dom`); task 2
    before task 3 so the moved boot is linted (once `csr` is wasm-only, host
    clippy no longer sees it and only the extended `wasm-clippy` does — spec
    AC6). Task 2 lands green on its own (it lints today's `csr` on the wasm
    target).
  - `--features csr` with `-p web -p client -p csr`: `csr` has no `csr` feature,
    but cargo applies the flag to the selected packages that define it
    (`web`/`client`), and `csr` pulls `web[csr]` via its own dependency — so the
    combined invocation resolves. Verified by running `wasm-clippy` in task 2.
- **For agentic workers:** execute with **`jaunder-iterate`**; inline execution
  is appropriate (small, well-bounded). Delegate via **`jaunder-dispatch`** only
  if useful.

## Global Constraints

- Follow `CONTRIBUTING.md` (coverage, import discipline, ADR-0069 client
  charter: no domain types in `client`). No `Co-Authored-By` trailer.
- Per-commit gate `cargo xtask check` green before commit
  (**`jaunder-commit`**); the pre-commit hook enforces it. Wasm-only code is not
  linted by host clippy — the gate's `wasm-clippy` step covers
  `web`/`client`/`csr` on the wasm target.
- Review base: `git diff wt-base-issue-519..HEAD` (three-dot `main...HEAD`).

---

## Task 1 — `client::dom` primitives (AC2)

**Files:** `client/src/dom.rs` (new), `client/src/lib.rs`, `client/Cargo.toml`.

**Step 1a — new module `client/src/dom.rs`:**

```rust
//! Generic browser DOM primitives — "text content of element by id" and "remove
//! element by id". Raw `web_sys`, no domain types (the `navigation`/`dialog`
//! precedent, ADR-0069). The CSR boot (`csr`) reads the projector's seed blob and
//! drops the server-painted container through these.

/// The `text_content` of the element with `id`, if the element exists.
#[must_use]
pub fn text_content_by_id(id: &str) -> Option<String> {
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .text_content()
}

/// Remove the element with `id` from the document if present; no-op otherwise.
pub fn remove_element_by_id(id: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    {
        el.remove();
    }
}
```

**Step 1b — declare it** in `client/src/lib.rs`, unconditional (like
`navigation`/ `dialog`, not behind `csr` — uses only `web_sys`):

```rust
/// Generic browser DOM primitives (`text_content_by_id`, `remove_element_by_id`) —
/// raw `web_sys`, no domain types. The CSR boot reads the projector seed blob and
/// drops the server-painted `#app` through these (#519).
pub mod dom;
```

**Step 1c — web-sys features** in `client/Cargo.toml`: add `Document`,
`Element`, `Node` to the `web-sys` `features` list (currently
`Window`/`Storage`/`Location`) — `document()`, `get_element_by_id()`,
`text_content()`, and `Element::remove()` need them. (The compiler forces this;
add them up front.)

**Verify:**

```
cargo clippy -p client --features csr --target wasm32-unknown-unknown -- -D warnings
```

→ **PASS** (client compiles on wasm; the two `pub` primitives are not dead). The
host build ignores `client` entirely (crate-level
`#![cfg(target_arch = "wasm32")]`).

**Commit** (after `cargo xtask check` green):
`feat(client): add dom primitives (text_content_by_id, remove_element_by_id) (#519)`

---

## Task 2 — Extend `wasm-clippy` to lint `-p csr` (AC6)

**Files:** `xtask/src/steps/static_checks.rs`, `flake.nix`.

**Step 2a — `static_checks.rs` StepSpec** (the `"wasm-clippy"` args vec): insert
`"-p", "csr",` after the `client` pair, so the args read
`clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- …`.
Also extend the step's lead comment (`static_checks.rs:58-74`, which today
explains wasm-clippy as linting `web`/`client` on the wasm target) to name `csr`
— the wasm-only entry crate — as a third linted package
(docs-track-late-changes).

**Step 2b — the unit test:** update `wasm_clippy_lints_web_and_client` (the
`assert_eq!(wasm_clippy.args, [...])` block) to include the new `"-p", "csr",`
entries, and rename it `wasm_clippy_lints_web_client_and_csr`.

**Step 2c — `flake.nix` `wasm-clippy` derivation** (~lines 1113-1129): add
`-p csr` to both the `buildDepsOnly` `cargoExtraArgs` and the
`cargoClippyExtraArgs` (`"-p web -p client -p csr --features csr …"`). Update
the derivation's lead comment to note `csr` (the wasm-only entry) is now linted
here too, alongside `web::pages`.

**Verify:**

```
cargo test -p xtask --lib steps::static_checks
```

→ **PASS** (the renamed test asserts the new package set).

```
devtool run -- cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations
```

→ **PASS** — confirms the combined `-p csr` invocation resolves and today's
`csr` lints clean on wasm. (If `--features csr` errors on `csr`, fall back to
relying on `csr`'s own `web[csr]` dep — resolve during implementation and note
it.)

**Commit:** `build(xtask): wasm-clippy lints -p csr (#519)`

---

## Task 3 — Move the boot into `csr`; make `csr` wasm-only; strip `web/src/lib.rs` (AC1, AC3, AC4, AC5)

One atomic commit — the `web` strip and the `csr` boot are interdependent.

**Files:** `csr/src/lib.rs`, `csr/Cargo.toml`, `web/src/lib.rs`.

**Step 3a — `csr/Cargo.toml` deps:** add `client = { path = "../client" }` and
`serde_json.workspace = true`.

**Step 3b — rewrite `csr/src/lib.rs`** as wasm-only with the boot:

```rust
#![cfg(target_arch = "wasm32")]
// web::App's ParentRoute generates a wide route tuple; raise the recursion limit
// to monomorphize it (mirrors web/src/lib.rs).
#![recursion_limit = "512"]

use leptos::prelude::*;
use web::render::PageSeed;
use web::App;

// The e2e suite waits on `body[data-hydrated]` (end2end/tests/hydration.ts) as the
// "app is mounted and interactive" signal. CSR has no hydration, but the same marker
// cleanly means "mount_to_body done" here, so the specs need no changes.
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_ready() {
        if (document && document.body) {
            document.body.setAttribute('data-hydrated', 'true');
        }
    }
")]
extern "C" {
    fn mark_ready();
}

/// Boot the CSR client (#179). Adopts the public projector's data blob (#178):
/// reads `#jaunder-seed`, drops the projector-painted `#app` container, and mounts
/// [`App`] with the seed in context so the public pages render their first paint from
/// it (no reactive fetch) via the same `render` fn the projector used — coincident,
/// flash-free. On the static SPA shell (no blob, no `#app`) the seed is `None` and
/// this is an ordinary `mount_to_body`.
fn mount() {
    let seed = client::dom::text_content_by_id("jaunder-seed")
        .and_then(|json| serde_json::from_str::<PageSeed>(&json).ok());
    // App re-renders the identical content from `seed`, so removing the
    // server-painted copy avoids a duplicate paint without a visible flash (the
    // removal and remount happen in one synchronous task).
    client::dom::remove_element_by_id("app");
    leptos::mount::mount_to_body(move || {
        provide_context(seed.clone());
        view! { <App /> }
    });
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();
    mount();
    mark_ready();
}
```

No `cov:ignore` anywhere (the crate is empty on host, so nothing is
coverage-measured).

**Step 3c — strip `web/src/lib.rs`:** delete, as one contiguous block, the
**comment at lines 57-58**
("`// Only the wasm32 body of mount_csr below uses the leptos prelude…`" — it
literally contains the string `mount_csr`, so leaving it would fail AC1's `rg`),
the
`#[cfg(all(feature = "csr", target_arch = "wasm32"))] use leptos::prelude::*;`
line (59-60), `mount_csr` with its `cov:ignore` markers (62-93), and
`read_dom_seed` (95-103). **Keep**
`#[cfg(target_arch = "wasm32")] pub mod pages;` and
`#[cfg(target_arch = "wasm32")] pub use pages::App;` unchanged.

**Verify:**

```
rg -n 'mount_csr|read_dom_seed|cov:ignore|feature = "csr"' web/src/lib.rs
```

→ **no matches** (AC1). And
`git diff wt-base-issue-519 -- web/src/render/mod.rs` is **empty** (AC3 —
`web::render` untouched).

```
cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations
```

→ **PASS** — the moved boot is clippy-clean on wasm (AC4/AC6). A host build
(`cargo check --workspace`) shows `csr` empty on host (no `cov:ignore` needed).

**Commit:** `refactor(web): move the CSR boot to csr; make csr wasm-only (#519)`

---

## Task 4 — Full gate (AC5, AC7)

**Verify:**

```
cargo xtask validate
```

→ **PASS** — static + clippy + `wasm-clippy` (now incl. `csr`) + coverage
(clean, no new `cov:ignore`) + the **e2e matrix**, which exercises both boot
paths: seed adoption on anon public `/` (`authed-flash.spec.ts`) and the
static-shell mount on authed drafts/composer (`posts.spec.ts`). A green e2e is
the AC5 behavior-preservation proof (every page load runs the boot and waits on
`body[data-hydrated]`).

Then confirm a clean tree:

```
git status --porcelain
```

→ empty (three commits: client dom, wasm-clippy, the move).

## Self-review

- Every spec AC maps to a task: AC1/AC3/AC4/AC5 → task 3; AC2 → task 1; AC6 →
  task 2 (+ task 3 lint run); AC7 → task 4. The "no new e2e / no web::render
  surface" non-goals are honored (no such task).
- Tasks independently verifiable, ordered (1 → 2 → 3 → 4), each with an explicit
  expected result; task 3 is one atomic commit by necessity.
- No task smuggles out-of-scope work; `web::render`/`PageSeed` untouched (that's
  #610).

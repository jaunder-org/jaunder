# Plan — #322: converge the `password_reset` vertical onto the file-level host/wasm split

**Spec:**
`docs/superpowers/specs/2026-07-22-issue-322-web-password-reset-colocate.md`
(problem/why/decisions live there — this plan is "how"). **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (as amended by #530,
#527). **For agentic workers:** drive with `jaunder-iterate` (delegate a task to
`jaunder-dispatch` when useful); tick checkboxes in real time.

## Review header

**Goal.** Move the `password_reset` vertical to the ADR-0070 file-level split —
`mod.rs` (wiring) / `api.rs` (the two `#[server]` fns) / `component.rs`
(wasm-only UI) — merging the two `#[cfg(feature="server")]` use-blocks into one.
No `server.rs`, no `status.rs`, no extraction, no SSR vestige. No behavior
change.

**Scope.**

- _In:_ the three `web/src/password_reset/*` files; merging the double
  cfg(server) block; repointing `Topbar` to `crate::topbar`; rewiring
  `pages/mod.rs`; the stale-issue-text fix on #322.
- _Out:_ #330 (App/Router move), #312 (`pages/ui.rs` dissolve), #306 (guard),
  any change to reset behavior / wire contract / secret model / enumeration
  semantics / mailer / storage.

**Tasks.**

1. Extract the two `#[server]` fns into `password_reset/api.rs` (merging the two
   cfg(server) blocks into one); `mod.rs` wires + re-exports them.
2. Cutover: move the UI into wasm-only `password_reset/component.rs` (repoint
   `Topbar`, preserve the `with_untracked` token read), delete
   `pages/password_reset.rs`, rewire the router. (The username-enumeration
   avoidance lives in the `request_password_reset` server body, so it travels
   with the Task 1 `api.rs` move.)
3. Full gate + conformance check.
4. Issue hygiene: fix #322's stale "Blocked by #312" text.

**Amendment tasks (2026-07-22 — see spec "Scope amendment").**

5. `password_reset/api.rs`: compose an **absolute** reset link from
   `site.base_url` (add `SiteConfigStorage` + `compose`, error if unset).
6. `email/api.rs`: the same absolute-link fix for `request_email_verification`.
7. Real e2e: `mail.ts` `extractLink`; `password_reset.spec.ts` +
   `email.spec.ts` assert the link is absolute, then follow its path
   (`new URL(link)` origin-strip + `goto`). `atompub.spec`/`helpers.ts` unchanged.
8. Re-gate + re-review + file the username→email forgot-password enhancement as
   its own issue.

**Key risks / decisions.**

- The cutover (task 2) is atomic — component move + `pages/password_reset.rs`
  delete + router repoint must land in one commit or the crate won't compile in
  between.
- Components are now **wasm-only** ⇒ `wasm-clippy` (`-p web`) is load-bearing;
  host `cargo check` alone won't type-check the UI.
- The wire-struct re-exports (`RequestPasswordReset`, `ConfirmPasswordReset`)
  are load-bearing for the **server integration-test registrar**
  (`server/tests/helpers/mod.rs`), not just `-p web` — the full
  `check`/`validate` gate (which builds the server integration tests) covers
  this; a bare `cargo check -p web` would not.
- No `status.rs`: ADR-0070 §6 found no extractable pure logic.

## Global constraints

- **Verbatim moves.** The `#[server]` bodies and the component `view!` trees
  move unchanged except the spec-authorized edits: `Topbar` path, merging the
  cfg(server) blocks, and cfg placement. Preserve the username-enumeration
  avoidance and the `with_untracked` token-read + race comment exactly.
- **Gate.** Pre-commit hook runs `cargo xtask check` (fmt + clippy + Nix
  coverage/tests incl. the server integration tests + `wasm-clippy`). Run
  `cargo xtask check` green before each commit (`jaunder-commit`). **No
  `Co-Authored-By` trailer.**
- **e2e:** run `cargo xtask validate --no-e2e` (local e2e VM reaped → CI matrix
  gates the four `{sqlite,postgres}×{chromium,firefox}` combos).
- **ADR-0070 invariants:** `target_arch = "wasm32"` only on `mod` declarations /
  paired `pub use`; zero cfgs inside `component.rs`; **exactly one grouped
  `#[cfg(feature="server")]` block** in `api.rs`; no `cov:ignore` /
  `#[component]`-exemption; no fake host stub (ADR-0055).

---

## Task 1 — extract `#[server]` fns into `password_reset/api.rs` (merge cfg blocks)

**Why:** ADR-0070 (#530): endpoints live in `api.rs`, `mod.rs` is wiring; and
§1's one-grouped-support-gate rule (merge the two cfg(server) blocks). Pure
move; keeps `pages/password_reset.rs` compiling via re-exports.

**Files:**

- **New `web/src/password_reset/api.rs`:** move both `#[server]` fns
  (`request_password_reset`, `confirm_password_reset`) verbatim from
  `password_reset/mod.rs`. **Merge** the two `#[cfg(feature = "server")]`
  use-blocks into one grouped block — fold `crate::error::InternalError` into
  the main block:
  ```rust
  #[cfg(feature = "server")]
  use {
      chrono::Duration,
      common::mailer::{EmailMessage, MailSender},
      common::password::Password,
      crate::error::InternalError,
      std::sync::Arc,
      storage::{AtomicOps, PasswordResetStorage, UserStorage},
  };
  ```
  Keep the ungated wire-arg imports ungated (`crate::error::WebResult`,
  `common::password::ProfferedPassword`, `common::token::RawToken`,
  `common::username::Username`, `leptos::prelude::*`). Add a `//!` doc:
  `//! Password-reset vertical — API surface: the reset #[server] endpoints and their wire arg types.`
  (leptosfmt/rustfmt will order the merged `use {}` block; write it readably and
  let fmt settle it.)
- **Rewrite `web/src/password_reset/mod.rs`** to wiring only (this task lands
  `mod api;` + the `pub use api::{…}` line; the `component` lines arrive in task
  2):

  ```rust
  //! Password-reset vertical — module wiring (ADR-0070). API surface in
  //! `api.rs`, the wasm-only UI in `component.rs`.
  mod api;

  pub use api::{
      confirm_password_reset, request_password_reset, ConfirmPasswordReset, RequestPasswordReset,
  };
  ```

**Verify (pure move):**

- `cargo xtask check --no-test` → green (fmt + host clippy + wasm-clippy).
- `cargo check -p web --all-features` → green.
- Grep sanity: `rg -c 'cfg\(feature = "server"\)' web/src/password_reset/api.rs`
  → exactly `1`; `rg 'server\)]' web/src/password_reset/mod.rs` → nothing;
  `pages/password_reset.rs` still imports
  `crate::password_reset::{ConfirmPasswordReset, RequestPasswordReset}` and
  resolves.

**Commit:**
`refactor(web/password_reset): move #[server] fns into api.rs; merge cfg(server) blocks; mod.rs wiring`

---

## Task 2 — cutover: wasm-only `component.rs`, rewire router

**Why:** co-location + gating. Atomic (crate won't compile mid-way).

**Files:**

- **New `web/src/password_reset/component.rs`** (`#[cfg(target_arch="wasm32")]`
  on the `mod` line; **zero cfgs inside**): move `ForgotPasswordPage` +
  `ResetPasswordPage` from `pages/password_reset.rs`, with these edits:
  - `use crate::topbar::Topbar;` (not `crate::pages::Topbar`); the vertical wire
    structs via `use super::{ConfirmPasswordReset, RequestPasswordReset};`; keep
    `crate::error::WebError`, `crate::forms::{Field, ValidatedInput}`,
    `common::password::Password`, `common::username::Username`,
    `leptos::prelude::*`, and `leptos_router::components::Redirect`.
  - **Preserve verbatim:** the `use_query_map().with_untracked(...)` token read
    and its race comment (`ResetPasswordPage`), and the neutral-message /
    error-mapping render arms.
  - Add a `//!` doc:
    `//! Password-reset vertical — wasm-only UI (ADR-0070): the forgot-password and reset-password pages.`
- **`web/src/password_reset/mod.rs`:** add
  `#[cfg(target_arch = "wasm32")] mod component;` and
  `#[cfg(target_arch = "wasm32")] pub use component::{ForgotPasswordPage, ResetPasswordPage};`.
- **Delete `web/src/pages/password_reset.rs`.**
- **`web/src/pages/mod.rs`:** delete `pub mod password_reset;` (line 1); move
  the router import out of the `crate::pages::` group to the `crate::` vertical
  group — `use crate::password_reset::{ForgotPasswordPage, ResetPasswordPage};`
  (alphabetically after `pages`/before `posts` in the vertical group — place so
  rustfmt won't reflow). The two `<Route>` lines are unchanged.

**Verify:**

- `cargo xtask check --no-test` → green — **wasm-clippy is the load-bearing
  check** (UI now wasm-only).
- `rg 'crate::pages::Topbar' web/src/password_reset/` → none;
  `rg -n '#\[cfg' web/src/password_reset/component.rs` → only `//!`/doc text, no
  real attribute.
- `rg 'with_untracked' web/src/password_reset/component.rs` → present (race read
  preserved).
- `rg 'mod password_reset' web/src/pages/mod.rs` → no `pub mod password_reset;`;
  `web/src/pages/password_reset.rs` no longer exists.

**Commit:**
`refactor(web/password_reset): move UI into wasm-only component.rs; rewire router`

---

## Task 3 — full gate + conformance check

**Why:** prove the acceptance floor.

**Steps:**

- `cargo xtask validate --no-e2e` → green (static + wasm-clippy + coverage +
  server integration tests — the registrar consumer). Read
  `.xtask/last-result.json` `steps[]` if anything is not `ok`.
- Walk the spec's 12 acceptance criteria and confirm each observably (file set =
  `mod.rs`/`api.rs`/`component.rs`, no `server.rs`/`status.rs`; exactly one
  cfg(server) block; no `crate::pages::Topbar`; router repointed; wire structs
  re-exported; `with_untracked` + enumeration logic intact).
- **e2e:** local VM reaped — do not run locally. The four
  `password_reset.spec.ts` flows gate in CI; note this in the ship PR body.
- `git status --porcelain` after green — stage/commit any fmt residue.

**Commit (if any residue):** `chore(web/password_reset): gate fixups`

---

## Task 4 — issue hygiene: fix #322's stale "Blocked by #312" text

**Why:** spec Decision 7. #322's body says "Blocked by the prereq (#312)"; the
native graph is the reverse. Not code — `gh issue edit` at ship time (same
correction landed on #318/#320). Leave the re-scope notes intact. No commit.

---

## Self-review

- Every spec acceptance criterion maps to a task: 1 (api split, cfg merge, wire
  re-exports, no server.rs), 2 (component gating, Topbar, router, no cfg inside,
  preserved verbatim details), 3 (gate incl. integration tests, no-status.rs,
  no-stub, conformance), 4 (issue text).
- Tasks ordered so the crate compiles after each (task 1 keeps
  `pages/password_reset.rs` alive via re-exports; task 2 is the atomic cutover).
- No task smuggles out-of-scope work (#330/#312/#306 excluded).
- No placeholders: exact files, exact edits, exact `cargo`/`gh` commands.

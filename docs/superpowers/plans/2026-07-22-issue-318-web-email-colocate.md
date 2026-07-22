# Plan — #318: converge the `email` vertical onto the file-level host/wasm split

**Spec:** `docs/superpowers/specs/2026-07-22-issue-318-web-email-colocate.md`
(problem/why/decisions live there — this plan is "how"). **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (as amended by #530,
#527). **For agentic workers:** drive with `jaunder-iterate` (delegate a task to
`jaunder-dispatch` when useful); tick checkboxes in real time.

## Review header

**Goal.** Move the `email` vertical from the pre-0070 shape (server fns in
`mod.rs`, components in `pages/email.rs`) to the ADR-0070 file-level split:
`mod.rs` (wiring) / `api.rs` (`#[server]` fns) / `component.rs` (wasm-only UI) /
`status.rs` (new, ungated host-tested pure logic). No `server.rs` (none
warranted). No behavior change to the email flows.

**Scope.**

- _In:_ the four `web/src/email/*` files; the two pure extractions
  (`email_status_line`, `parse_verification_token`) with `nextest` tests;
  repointing `Topbar` to `crate::topbar`; rewiring `web/src/pages/mod.rs`
  (delete `pub mod email;` + `pages/email.rs`, repoint router import); the
  stale-issue-text fix on #318.
- _Out:_ #330 (App/Router move), #312 (`pages/ui.rs`/`web::render` dissolve),
  #306 (automated thin-component guard), any change to email verification
  behavior / wire contract / mailer / storage.

**Tasks.**

1. Extract the two `#[server]` fns into `email/api.rs`; `mod.rs` wires +
   re-exports them.
2. Add `email/status.rs`: `email_status_line` + `parse_verification_token`,
   host-tested (TDD).
3. Cutover: move the components into wasm-only `email/component.rs`, delete
   `pages/email.rs`, rewire the router.
4. Full gate + conformance check against the spec's acceptance floor.
5. Issue hygiene: fix #318's stale "Blocked by #312" text.

**Key risks / decisions.**

- The cutover (task 3) is atomic — splitting the component move from the
  `pages/email.rs` delete + router repoint leaves a non-compiling intermediate,
  so they land in one commit.
- Components are now **wasm-only** ⇒ `wasm-clippy` (`-p web`) is load-bearing; a
  host `cargo check` alone will not catch UI type errors. Every task that
  touches `component.rs` must run the wasm-clippy step.
- `status.rs` fns are called only from the wasm-only `component.rs`, so they'd
  be host `dead_code` if not `pub use`-re-exported from `mod.rs` — the
  `media::extract_upload_url` precedent. The re-export is part of the wiring in
  task 1/2, not deferred.

## Global constraints

- **Verbatim moves.** The `#[server]` bodies and the component `view!` trees
  move unchanged except the three edits the spec authorizes: `Topbar` path,
  `status::` calls replacing the two inlined branches, and cfg placement. No
  redesign of the email flows.
- **Gate.** The pre-commit hook runs `cargo xtask check` (fmt + clippy + Nix
  coverage/tests, incl. `wasm-clippy`). Run `cargo xtask check` green before
  each commit (`jaunder-commit`). Host-side server-gated code needs
  `--all-features`; the UI needs the wasm target — `cargo xtask check` covers
  both. **No `Co-Authored-By` trailer.**
- **Local e2e is reaped here** — run `cargo xtask validate --no-e2e`; the CI
  matrix gates the four `{sqlite,postgres}×{chromium,firefox}` e2e combos at
  ship.
- **ADR-0070 invariants:** `target_arch = "wasm32"` only on `mod` declarations /
  paired `pub use`, never inside a leaf file; zero cfgs inside `component.rs`;
  no `cov:ignore` / `#[component]`-exemption reliance; no fake host stub
  (ADR-0055).

---

## Task 1 — extract `#[server]` fns into `email/api.rs`; `mod.rs` becomes wiring

**Why:** ADR-0070 (#530): endpoints live in `api.rs`, `mod.rs` is wiring only.
Pure move, no behavior change; keeps `pages/email.rs` compiling via the
re-exports.

**Files:**

- **New `web/src/email/api.rs`:** move, verbatim, from the current
  `web/src/email/mod.rs`: the leading `#[cfg(feature = "server")] use { … }`
  block (lines 1–8), the ungated `use` lines it needs
  (`crate::error::WebResult`, `common::email::Email`, `common::token::RawToken`,
  `leptos::prelude::*`), and both `#[server]` fns (`request_email_verification`,
  `verify_email`) with their doc comments. Add an ADR-0070-style `//!` doc:
  `//! Email vertical — API surface: verification `#[server]` endpoints and their wire types.`
- **Rewrite `web/src/email/mod.rs`** to wiring only:

  ```rust
  //! Email vertical — module wiring (ADR-0070). API surface in `api.rs`, the
  //! wasm-only UI in `component.rs`, pure host-tested helpers in `status.rs`.
  mod api;
  mod status;
  #[cfg(target_arch = "wasm32")]
  mod component;

  pub use api::{
      request_email_verification, verify_email, RequestEmailVerification, VerifyEmail,
  };
  pub use status::{email_status_line, parse_verification_token};
  #[cfg(target_arch = "wasm32")]
  pub use component::{EmailPage, VerifyEmailPage};
  ```

  (`status`/`component` don't exist until tasks 2–3 — so land the `mod status;`
  - its `pub use`, and the `component` lines, in those tasks. In **this** task,
    `mod.rs` declares only `mod api;` + the `pub use api::{…}` line; add the
    `status`/`component` wiring as those files arrive.)

**Verify (no new test — pure move):**

- `cargo xtask check --no-test` → green (fmt + host clippy + wasm-clippy).
- `cargo check -p web --all-features` → green (server-gated bodies compile).
- Grep sanity: `rg 'server\)]' web/src/email/mod.rs` returns nothing (no
  endpoints left in `mod.rs`); `pages/email.rs` still imports
  `crate::email::{verify_email, RequestEmailVerification}` and resolves.

**Commit:**
`refactor(web/email): move #[server] fns into api.rs; mod.rs wiring only`

---

## Task 2 — `email/status.rs`: pure host-tested helpers (TDD)

**Why:** ADR-0070 §6 — extract the two pure branches out of the wasm-only
component into ungated, host-tested, coverage-measured code _before_ gating.

**Files:**

- **New `web/src/email/status.rs`** (cfg-free, host-compiled):

  ```rust
  //! Email vertical — pure, host-tested helpers extracted from the UI (ADR-0070
  //! §6): status-line formatting and verification-token parsing.
  use crate::error::WebError;
  use common::email::Email;
  use common::token::RawToken;

  /// Formats the account's current email-verification status for display.
  pub fn email_status_line(email: Option<&Email>, verified: bool) -> String {
      match (email, verified) {
          (Some(e), true) => format!("{e} (verified)"),
          (Some(e), false) => format!("{e} (unverified)"),
          (None, _) => "No email set".to_string(),
      }
  }

  /// Parses a raw verification token, mapping a malformed value to a client-side
  /// validation error (ADR-0065 pre-validation) rather than a server round-trip.
  pub fn parse_verification_token(raw: &str) -> Result<RawToken, WebError> {
      raw.parse()
          .map_err(|_| WebError::validation("invalid verification token"))
  }
  ```

- **`web/src/email/mod.rs`:** add `mod status;` and
  `pub use status::{email_status_line, parse_verification_token};` (the
  re-export prevents host `dead_code` — the only non-test caller is the
  wasm-only component, arriving in task 3).

**Test — in-file `#[cfg(test)] mod tests`** (web uses in-file unit tests):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_email;

    #[test]
    fn status_line_verified() {
        assert_eq!(
            email_status_line(Some(&parse_email("a@b.com")), true),
            "a@b.com (verified)"
        );
    }
    #[test]
    fn status_line_unverified() {
        assert_eq!(
            email_status_line(Some(&parse_email("a@b.com")), false),
            "a@b.com (unverified)"
        );
    }
    #[test]
    fn status_line_none() {
        assert_eq!(email_status_line(None, false), "No email set");
        assert_eq!(email_status_line(None, true), "No email set");
    }
    #[test]
    fn parse_token_valid() {
        // `"abcABC012-_"` is the crate's own valid RawToken fixture (base64url,
        // no-pad, not length-pinned — common/src/token.rs).
        assert!(parse_verification_token("abcABC012-_").is_ok());
    }
    #[test]
    fn parse_token_invalid() {
        let err = parse_verification_token("not a token").unwrap_err();
        assert!(err.to_string().contains("invalid verification token"));
    }
}
```

> Test helpers route through `common::test_support::parse_email` (the newtype
> test-helper convention) — `web`'s dev-dependencies already enable common's
> `test-support` feature. The valid-token literal and the `"not a token"`
> rejection are both the crate's own fixtures (`common/src/token.rs:153,155`).

**Run:**

- `cargo nextest run -p web email::status` → **FAIL** first (write tests, then
  the impl, per TDD — or land impl+tests together and confirm PASS).
- After impl: `cargo nextest run -p web email::status` → **PASS** (5 tests).
- `cargo check -p web` (host, no `--all-features`) → green with **no
  `dead_code`** warning for the two fns (proves the re-export works).

**Commit:**
`feat(web/email): extract email_status_line + parse_verification_token (host-tested)`

---

## Task 3 — cutover: wasm-only `component.rs`, delete `pages/email.rs`, rewire router

**Why:** the co-location + gating step. Atomic: the component move, the
`pages/email.rs` delete, and the router repoint must land together or the crate
won't compile in between.

**Files:**

- **New `web/src/email/component.rs`** (`#[cfg(target_arch = "wasm32")]` on the
  `mod` line in `mod.rs`; **zero cfgs inside this file**): move `EmailPage` and
  `VerifyEmailPage` from `pages/email.rs`, with exactly these edits:
  - Import `use crate::topbar::Topbar;` (not `crate::pages::Topbar`).
  - In `EmailPage`, replace the inlined status match (`pages/email.rs:27–31`)
    with
    `crate::email::email_status_line(data.email.as_ref(), data.email_verified)`.
  - In `VerifyEmailPage`, replace the inlined `raw.parse()` + `map_err`
    (`pages/email.rs:84–88`) with
    `crate::email::parse_verification_token(&raw)?` inside the `server_resource`
    closure.
  - Add `//!` doc:
    `//! Email vertical — wasm-only UI (ADR-0070): the settings page and the verify-email landing.`
- **`web/src/email/mod.rs`:** add
  `#[cfg(target_arch = "wasm32")] mod component;` and
  `#[cfg(target_arch = "wasm32")] pub use component::{EmailPage, VerifyEmailPage};`
  (completing the wiring block from task 1).
- **Delete `web/src/pages/email.rs`.**
- **`web/src/pages/mod.rs`:** delete `pub mod email;` (line 1); change the
  router import `use crate::pages::email::{EmailPage, VerifyEmailPage};`
  (line 26) → `use crate::email::{EmailPage, VerifyEmailPage};`. The two
  `<Route>` lines (`profile/email` → `EmailPage`, `verify-email` →
  `VerifyEmailPage`) are unchanged.

**Verify:**

- `cargo xtask check --no-test` → green — **wasm-clippy is the load-bearing
  check** here (the UI is now wasm-only; host clippy won't type-check it).
- `rg 'crate::pages::Topbar' web/src/email/` → no matches;
  `rg -n '#\[cfg' web/src/email/component.rs` → matches only in `//!`/doc text,
  never a real attribute.
- `rg 'mod email' web/src/pages/mod.rs` → no `pub mod email;` remains;
  `rg 'crate::email::\{EmailPage' web/src/pages/mod.rs` → present.
- Confirm `web/src/pages/email.rs` no longer exists.

**Commit:**
`refactor(web/email): move UI into wasm-only component.rs; rewire router`

---

## Task 4 — full gate + conformance check

**Why:** prove the acceptance floor; catch anything the per-task checks missed.

**Steps:**

- `cargo xtask validate --no-e2e` → green (static + wasm-clippy + coverage).
  Read `.xtask/last-result.json` `steps[]` if anything is not `ok`.
- Walk the spec's 12 acceptance criteria and confirm each observably (file set =
  `mod.rs`/`api.rs`/`component.rs`/`status.rs`, no `server.rs`; no
  `crate::pages::Topbar`; router import repointed; `nextest` green; no host
  `dead_code`; cfg placement clean).
- **e2e:** local VM is reaped — do **not** attempt the local e2e run. The three
  `end2end/tests/email.spec.ts` flows are structurally unaffected (same
  selectors, flash strings, routes, and `/verify-email?token=` handling); they
  gate in CI at ship. Note this explicitly in the ship PR body.
- `git status --porcelain` after the green gate — `cargo xtask check` may have
  auto-fixed fmt; stage/commit any residue.

**Commit (if any residue):** `chore(web/email): gate fixups`

---

## Task 5 — issue hygiene: fix #318's stale "Blocked by #312" text

**Why:** the spec's tracker-hygiene decision. #318's body says "Blocked by the
prereq (#312)", but the native graph is the reverse (#312 is `blocked_by` every
vertical) and four verticals shipped while #312 is open. Not code — do it at
ship time (or now) with `gh issue edit`.

**Steps:**

- Edit #318's body to correct/remove the "Blocked by the prereq (#312)" line
  (replace with a note that #312 is the downstream omnibus, `blocked_by` this
  vertical). Leave the re-scope notes intact.
- No code, no commit.

---

## Self-review

- Every spec acceptance criterion maps to a task: 1–2 (file layout, api split,
  status extraction, dead_code), 3 (component gating, Topbar, router, delete), 4
  (gate, e2e-in-CI, no-stub), 5 (issue text).
- Tasks are ordered so the crate compiles after each (task 1 keeps
  `pages/email.rs` alive via re-exports; task 3 is the atomic cutover).
- No task smuggles out-of-scope work (#330/#312/#306 explicitly excluded).
- No placeholders: exact files, exact `cargo`/`gh` commands, exact edits.

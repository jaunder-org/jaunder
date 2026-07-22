# Spec — #318: converge the `email` vertical onto the file-level host/wasm split

**Status:** awaiting approval. **Parent:** #303 (umbrella). **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (supersedes ADR-0056),
as amended by #530 (`api.rs` split) and #527 (shared leaves as top-level
`crate::topbar` etc.). No new ADR — this vertical is a straight application of
ADR-0070's established pattern (`auth` #315 is the template; `media`, `posts`,
`timeline`, `home` already landed under it).

## Problem

The `email` vertical is split across files by _technology_, not feature, and its
UI still lives in the old `pages/` home:

- `web/src/email/mod.rs` — the two `#[server]` fns
  (`request_email_verification`, `verify_email`) inline, with their grouped
  `#[cfg(feature = "server")]` support-import block. This is the pre-0070 shape:
  endpoints live in `mod.rs` instead of a dedicated `api.rs`.
- `web/src/pages/email.rs` — the two `#[component]`s (`EmailPage`,
  `VerifyEmailPage`), importing `Topbar` from the stale `crate::pages::`
  re-export.

The UI and the server fns of one feature live in separate homes, and two pieces
of pure logic sit _inside_ the components — which are already wasm-only (they
ride the `#[cfg(target_arch = "wasm32")]` gate on `pub mod pages` in
`web/src/lib.rs`), so those branches never host-compile and are invisible to the
host coverage gate:

- `EmailPage`'s status line — `(Option<Email>, verified: bool)` →
  `"<e> (verified)"` / `"<e> (unverified)"` / `"No email set"`.
- `VerifyEmailPage`'s token parse — `raw.parse::<RawToken>()` mapped to a
  `WebError::validation("invalid verification token")`.

## Decisions (interview-resolved)

1. **Three-file layout, no `server.rs`.** The `#[server]` bodies touch only
   crate/external imports (`crate::auth::require_auth`,
   `crate::error::InternalError`, `common::mailer::{EmailMessage, MailSender}`,
   `std::sync::Arc`, `storage::{EmailVerificationStorage, UserStorage}`) — there
   is **no email-specific host-only helper**, so per ADR-0070 `server.rs` is
   omitted and the grouped `#[cfg(feature = "server")]` use-block sits inline in
   `api.rs` (the `media` pattern). Final shape: `mod.rs` / `api.rs` /
   `component.rs` + the new pure `status.rs` (Decision 3).
2. **UI moves into wasm-only `component.rs`.** `EmailPage` and `VerifyEmailPage`
   move to `web/src/email/component.rs`, declared
   `#[cfg(target_arch = "wasm32")] mod component;` in `email/mod.rs` with the
   matching gated re-export
   `#[cfg(target_arch = "wasm32")] pub use component::{EmailPage, VerifyEmailPage};`.
   The file carries **zero cfg gates inside it**, does not host-compile, and is
   **not** dead-but-exempt — no `cov:ignore` and no `#[component]`-exemption
   reliance is added.
3. **Extract both pure branches into an ungated, host-tested `status.rs`
   (ADR-0070 §6), _before_ gating the component.** A new
   `web/src/email/status.rs` — cfg-free, host-compiled, coverage-measured —
   holds:
   - `email_status_line(email: Option<&Email>, verified: bool) -> String` — the
     status formatting, all three arms.
   - `parse_verification_token(raw: &str) -> Result<RawToken, WebError>` — the
     token parse + validation-error mapping.

   Each is covered by `#[cfg(test)] mod tests` `nextest` assertions over every
   branch (both status transitions, and valid/invalid token). `component.rs`
   calls them instead of inlining the logic. Both are re-exported from `mod.rs`
   (ungated) so they are not host `dead_code` (the `media::extract_upload_url`
   precedent).

4. **`#[server]` fns + wire types move to `api.rs`, re-exported from a
   wiring-only `mod.rs`.** `request_email_verification` / `verify_email` and
   their generated wire structs (`RequestEmailVerification`, `VerifyEmail`) move
   verbatim to `web/src/email/api.rs`; `email/mod.rs` becomes module wiring +
   re-exports only, re-exporting those names explicitly — the `EmailPage`
   `ServerAction::<RequestEmailVerification>` call site depends on
   `crate::email::RequestEmailVerification` resolving — and keeping the
   registrar path `web::email::<Leaf>` stable.
5. **Repoint `Topbar` to its current home.** The components import
   `crate::topbar::Topbar` (the top-level shared-widget module, ADR-0070 #527),
   not the stale `crate::pages::Topbar` re-export.
6. **Fix the stale issue text on #318.** #318's body says "Blocked by the prereq
   (#312)", but the native dependency graph is the reverse (#312 is `blocked_by`
   every vertical, including #318) and four verticals shipped while #312 is
   still open. Correct the misleading line on the GitHub issue so the tracker
   matches reality. (Tracker hygiene, not code.)

## Target end state (acceptance floor)

Each criterion is observable so ship-time conformance review can tell delivered
from not:

1. `email`'s UI, `#[server]` fns, and wire types live under `web/src/email/`;
   **no `web/src/pages/email.rs` remains**, and its `pub mod email;` line in
   `web/src/pages/mod.rs` is deleted.
2. `web/src/email/` contains exactly `mod.rs`, `api.rs`, `component.rs`,
   `status.rs` — and **no `server.rs`** (none is warranted).
3. `email/mod.rs` is **wiring only**: `mod` declarations + `pub use` re-exports,
   no items of its own. `target_arch = "wasm32"` appears in the vertical **only
   on the `mod component;` declaration and its paired `pub use component::{…}`**
   — never on an item inside a file.
4. The `#[component]` UI lives in **wasm-only `component.rs`** with **zero cfg
   gates inside the file**; it does not host-compile and adds no `cov:ignore` /
   `#[component]`-exemption reliance.
5. `email/api.rs` carries the two `#[server]` fns (dual-compiled, ungated) with
   at most **one grouped `#[cfg(feature = "server")]` use-block** for the
   bodies. Their generated wire structs `RequestEmailVerification` and
   `VerifyEmail` are re-exported **by name** from `email/mod.rs` so
   `crate::email::RequestEmailVerification` (used by `EmailPage`) resolves.
6. `email/status.rs` is cfg-free, host-compiled, and holds `email_status_line`
   - `parse_verification_token`, each asserted by `nextest` over all branches;
     `component.rs` calls them rather than inlining the logic. The functions are
     reachable on host (re-exported) — the host build has **no `dead_code`
     warning** for them.
7. The router import in `web/src/pages/mod.rs` reads
   `use crate::email::{EmailPage, VerifyEmailPage};` (backed by the gated
   re-export in `email/mod.rs`); the two `<Route>` lines (`profile/email` →
   `EmailPage`, `verify-email` → `VerifyEmailPage`) are otherwise unchanged.
8. `EmailPage`/`VerifyEmailPage` import `crate::topbar::Topbar`; no
   `crate::pages::Topbar` reference remains in the email vertical.
9. No fake-value host stub is introduced (ADR-0055 principle).
10. `cargo xtask validate --no-e2e` green locally (static + `wasm-clippy` +
    coverage); the full e2e matrix green in CI. Because the components are now
    wasm-only, **`wasm-clippy` is load-bearing gate surface** for this
    vertical's UI type-checking, not just host clippy.
11. `end2end/tests/email.spec.ts` stays green — all three tests:
    - "email verification flow completes successfully" (login → `/profile/email`
      → submit new address → "Check your email" flash → extract token from
      mailbox → `/verify-email?token=…` → "verified" flash → reload shows
      "(verified)").
    - "email form gates submit until a valid address is entered" — pristine ⇒
      submit disabled, no error; invalid + blur ⇒ inline error, submit disabled;
      valid ⇒ error clears, submit enabled (#397 / ADR-0065).
    - "visiting verify-email with invalid token shows error".
12. #318's stale "Blocked by the prereq (#312)" line is corrected on GitHub.

## Shape of the work

- **`api.rs`** — cut the two `#[server]` fns + the `#[cfg(feature = "server")]`
  use-block from today's `email/mod.rs` into a new `email/api.rs`, unchanged.
- **`status.rs`** — extract `email_status_line` + `parse_verification_token`
  from the component bodies into a new cfg-free `email/status.rs` with `nextest`
  tests.
- **`component.rs`** — move `EmailPage`/`VerifyEmailPage` from `pages/email.rs`;
  repoint `Topbar` to `crate::topbar::Topbar`; replace the inlined status/token
  logic with calls to `status::{email_status_line, parse_verification_token}`;
  no cfg attributes inside.
- **`mod.rs`** — reduce to wiring: `mod api;`, `mod status;`,
  `#[cfg(target_arch = "wasm32")] mod component;`, plus the re-exports
  (`pub use api::{request_email_verification, verify_email, RequestEmailVerification, VerifyEmail};`,
  `pub use status::{email_status_line, parse_verification_token};`,
  `#[cfg(target_arch = "wasm32")] pub use component::{EmailPage, VerifyEmailPage};`).
  ADR-0070-style `//!` module doc.
- **Rewire.** `pages/mod.rs`: delete `pub mod email;`, delete `pages/email.rs`,
  change the router import to `crate::email::{EmailPage, VerifyEmailPage}`.
- **Issue hygiene.** Correct the stale "Blocked by #312" line on #318.

## Out of scope

- Moving `App`/Router out of `pages/mod.rs` to the app entry — that is **#330**.
- Dissolving `pages/ui.rs` / `web::render` — that is **#312**. This issue only
  stops _importing_ email's UI from `pages/`; it does not remove any remaining
  `pages/mod.rs` shim.
- Any change to the email verification behavior, wire contract, mailer, or the
  `EmailVerificationStorage` surface — the server fns move verbatim.
- The automated thin-component complexity guard — that is **#306**, sequenced to
  the end of the milestone. This issue does the manual ADR-0070 §6 extraction;
  #306 later automates catching regressions.

## Verification

`cargo xtask validate --no-e2e` locally (local e2e VM is reaped here; the CI
matrix gates the four `{sqlite,postgres}×{chromium,firefox}` e2e combos). The
load-bearing behavioral checks are the three `email.spec.ts` flows. The pure
extraction is asserted by `nextest` (`email_status_line`,
`parse_verification_token`). `wasm-clippy` (`-p web`) is load-bearing gate
surface for the now-wasm-only UI.

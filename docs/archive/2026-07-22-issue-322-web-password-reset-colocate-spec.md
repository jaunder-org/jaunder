# Spec — #322: converge the `password_reset` vertical onto the file-level host/wasm split

**Status:** awaiting approval. **Parent:** #303 (umbrella). **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (supersedes ADR-0056),
as amended by #530 (`api.rs` split) and #527 (shared leaves as top-level
`crate::topbar` etc.). No new ADR — a straight application of ADR-0070's
established pattern (`auth` #315 is the template; `email` #318, `invites` #320,
`media`, `posts`, `timeline`, `home` already landed under it).

## Scope amendment (2026-07-22) — email-link correctness

Maintainer-directed during ship review: the convergence surfaced a
**pre-existing bug** the verbatim move would have preserved.
`request_password_reset` builds a **relative** email link —
`format!("/reset-password?token={raw_token}")` — which is unusable in a real
mail client (no scheme/host). `invites` does it right
(`compose(&base_url, "/register")`); password_reset (and, identically, the
already-shipped `email` verification vertical) does not. The e2e never caught it
because it scrapes the `token` out of the email and rebuilds its own URL rather
than asserting the emitted link.

This amendment folds the fix into #322 (the issue explicitly invites
maintainer-directed API tightening within the vertical), **plus** the identical
fix in the `email` vertical (maintainer opted to fix the whole bug class here):

A. **`password_reset/api.rs` — absolute reset link.** Mirror `invites`: add
`SiteConfigStorage` to the server context, fetch `get_identity().base_url` **up
front** (error `"set the site base URL…"` if unset — so a misconfigured site
fails loudly rather than minting an orphan token + mailing a dead link), and
build `compose(&base_url, "/reset-password")` + `?token={raw_token}`. New
behavior: the flow now **requires** `site.base_url` (invites-consistent). B.
**`email/api.rs` — absolute verify link.** The same fix for
`request_email_verification` (`compose(&base_url, "/verify-email")`). C. **Real
e2e — follow the link, don't rebuild it.** `https://example.com` is a
_deliberately bogus_ base\*url (`seed_e2e.rs:39-42` — NOT the server's real
address; `atompub.spec`'s `onServer` re-bases such URLs onto the live server).
So the test must (1) `extractLink` (new `mail.ts` helper), (2) **assert the link
is absolute** (`^https://example.com/<path>?token=`) — the check that catches a
relative-link regression — and (3) **follow that link's own path** by stripping
the bogus origin (`new URL(link)`) and navigating its `pathname+search` via the
standard `goto` wrapper, rather than a URL rebuilt from the extracted token. (A
full-URL re-base helper does not fit `goto`, which prepends `BASE_URL`, so the
strip is inline; `atompub.spec` keeps its own `onServer`.) No serial-`-admin`
move: the canonical seed (`tools/devtool/src/seed_e2e.rs:43`) sets
`site.base_url = https://example.com` globally, so the flows work with no
per-test mutation and no global-singleton race.

The username-only limitation on the forgot-password form (should also accept an
email address) is a **separate feature** — filed as its own issue, out of scope
here.

## Problem

The `password_reset` vertical is split across files by _technology_, not
feature, and its UI still lives in the old `pages/` home:

- `web/src/password_reset/mod.rs` — the two `#[server]` fns
  (`request_password_reset`, `confirm_password_reset`) inline, with **two
  separate** `#[cfg(feature = "server")]` use-blocks (the main support block + a
  lone `use crate::error::InternalError;`). The pre-0070 shape: endpoints in
  `mod.rs` instead of a dedicated `api.rs`.
- `web/src/pages/password_reset.rs` — the two `#[component]`s
  (`ForgotPasswordPage`, `ResetPasswordPage`), importing `Topbar` from the stale
  `crate::pages::` re-export.

## Decisions (interview-resolved)

1. **Three-file layout, no `server.rs`, no `status.rs`, no extraction.** The
   `#[server]` bodies touch only crate/external imports (`chrono::Duration`,
   `common::mailer`, `common::password::{Password}`, `std::sync::Arc`,
   `storage::{AtomicOps, PasswordResetStorage, UserStorage}`,
   `crate::error::InternalError`, `host::metrics`) — no password_reset-specific
   host helper — so `server.rs` is omitted and the grouped
   `#[cfg(feature = "server")]` block sits inline in `api.rs` (the
   `media`/`email`/`invites` pattern). Per ADR-0070 §6 there is **no pure logic
   to extract**: both components are `view!`-building + `Field<T>` validation
   (which already lives in `crate::forms` / `common`); the token is passed as a
   raw hidden-input string to a typed `RawToken` wire arg (parsed at the serde
   boundary, **not** client-side). No SSR `set_status` vestige exists in either
   component. Final shape: `mod.rs` / `api.rs` / `component.rs`.
2. **Merge the two `#[cfg(feature = "server")]` use-blocks into one.** ADR-0070
   §1 wants **at most one grouped** support-import gate per vertical. Fold the
   lone `use crate::error::InternalError;` into the main grouped block in
   `api.rs`.
3. **UI moves into wasm-only `component.rs`.** `ForgotPasswordPage` +
   `ResetPasswordPage` move to `web/src/password_reset/component.rs`, declared
   `#[cfg(target_arch = "wasm32")] mod component;` in `password_reset/mod.rs`
   with the matching gated re-export
   `#[cfg(target_arch = "wasm32")] pub use component::{ForgotPasswordPage, ResetPasswordPage};`.
   The file carries **zero cfg gates inside it**, does not host-compile, and is
   **not** dead-but-exempt — no `cov:ignore` / `#[component]`-exemption
   reliance.
4. **Preserve two load-bearing details verbatim:** the username-enumeration
   avoidance in `request_password_reset` (same "contact operator" error whether
   the user is missing or lacks a verified email), and the token-read race
   comment + `use_query_map().with_untracked(...)` non-reactive read in
   `ResetPasswordPage` (a reactive read races WASM hydration and silently blanks
   the hidden token).
5. **`#[server]` fns + wire types move to `api.rs`, re-exported from a
   wiring-only `mod.rs`.** `request_password_reset` / `confirm_password_reset`
   and their generated wire structs (`RequestPasswordReset`,
   `ConfirmPasswordReset`) move verbatim to `web/src/password_reset/api.rs`;
   `password_reset/mod.rs` becomes wiring + re-exports only, re-exporting those
   names explicitly (the components' `ServerAction::<RequestPasswordReset>` /
   `ServerAction::<ConfirmPasswordReset>` call sites depend on them resolving),
   keeping the registrar path `web::password_reset::<Leaf>` stable.
6. **Repoint `Topbar`** to `crate::topbar::Topbar` (ADR-0070 #527), not the
   stale `crate::pages::Topbar` re-export.
7. **Fix the stale issue text on #322** — same as #318/#320: the body's "Blocked
   by the prereq (#312)" is reversed (native graph: #312 is `blocked_by` every
   vertical, and verticals shipped while #312 is open). Correct it on GitHub.

## Target end state (acceptance floor)

Each criterion is observable so ship-time conformance review can tell delivered
from not:

1. `password_reset`'s UI, `#[server]` fns, and wire types live under
   `web/src/password_reset/`; **no `web/src/pages/password_reset.rs` remains**,
   and its `pub mod password_reset;` line in `web/src/pages/mod.rs` is deleted.
2. `web/src/password_reset/` contains exactly `mod.rs`, `api.rs`, `component.rs`
   — and **no `server.rs`** and **no `status.rs`** (neither is warranted).
3. `password_reset/mod.rs` is **wiring only**: `mod` declarations + `pub use`
   re-exports, no items of its own. `target_arch = "wasm32"` appears in the
   vertical **only on the `mod component;` declaration and its paired
   `pub use component::{…}`** — never on an item inside a file.
4. The `#[component]` UI lives in **wasm-only `component.rs`** with **zero cfg
   gates inside the file**. It does not host-compile and adds no `cov:ignore` /
   `#[component]`-exemption reliance.
5. `password_reset/api.rs` carries the two `#[server]` fns (dual-compiled,
   ungated) with **exactly one** grouped `#[cfg(feature = "server")]` use-block
   (the two prior blocks merged). The ungated wire-arg imports (`WebResult`,
   `ProfferedPassword`, `RawToken`, `Username`, leptos prelude) stay ungated.
   Its wire structs `RequestPasswordReset` and `ConfirmPasswordReset` are
   re-exported **by name** from `password_reset/mod.rs` — this keeps
   `web::password_reset::{…}` resolving not only for the components but for the
   `#[server]` registrar (`server/tests/helpers/mod.rs` calls
   `register_explicit` on both), so a dropped re-export breaks the **server
   integration-test build**, not `-p web`.
6. `ForgotPasswordPage`/`ResetPasswordPage` import `crate::topbar::Topbar`; no
   `crate::pages::Topbar` reference remains in the vertical.
7. The router import in `web/src/pages/mod.rs` reads
   `use crate::password_reset::{ForgotPasswordPage, ResetPasswordPage};`; the
   two `<Route>` lines (`forgot-password` → `ForgotPasswordPage`,
   `reset-password` → `ResetPasswordPage`) are otherwise unchanged.
8. The username-enumeration avoidance and the `with_untracked` token-read (with
   its race comment) are preserved verbatim.
9. No fake-value host stub is introduced (ADR-0055 principle).
10. `cargo xtask validate --no-e2e` green locally (static + `wasm-clippy` +
    coverage); the full e2e matrix green in CI (local e2e VM is reaped here).
    Because the components are now wasm-only, **`wasm-clippy` is load-bearing
    gate surface** for this vertical's UI type-checking.
11. `end2end/tests/password_reset.spec.ts` stays green — all four tests:
    - "password reset flow completes successfully" (forgot form → neutral
      confirmation → reset link from mail → `/reset-password?token=…` → new
      password → redirect to `/login`; old password fails, new succeeds).
    - "visiting reset-password with invalid token shows error".
    - "reset-password rejects a too-short password client-side" (#410 —
      `Field<Password>` disables submit until valid).
    - "forgot-password for user without verified email shows contact operator
      error". Load-bearing selectors/strings preserved:
      `input[name="username"]`, `input[name="new_password"]`, the hidden `token`
      input, submit, `.error`, the neutral confirmation `<p>` (the e2e matches
      `/check|sent|email/i`), and the `/login` redirect.
12. #322's stale "Blocked by #312" line is corrected on GitHub.
13. **(Amendment A)** `request_password_reset` composes an **absolute** reset
    link from `site.base_url` (`compose(&base_url, "/reset-password")` +
    `?token=…`), fetched up front via `SiteConfigStorage` and erroring if unset.
    No relative `format!("/reset-password?…")` remains.
14. **(Amendment B)** `request_email_verification` (`email/api.rs`) composes an
    **absolute** verify link the same way
    (`compose(&base_url, "/verify-email")`). No relative
    `format!("/verify-email?…")` remains.
15. **(Amendment C)** `mail.ts` gains `extractLink`. Both
    `password_reset.spec.ts` (happy path) and `email.spec.ts` (verification
    flow) **assert the emitted link is absolute**
    (`^https://example.com/<path>?token=`) and then **follow that link's own
    path** — strip the bogus origin with `new URL(link)` and navigate its
    `pathname+search` via the `goto` wrapper — not a token-rebuilt URL. Both
    specs stay in the parallel project (base_url is pre-seeded — no `-admin`
    move, no race). The full e2e matrix stays green in CI.

## Shape of the work

- **`api.rs`** — move both `#[server]` fns from `password_reset/mod.rs`;
  **merge** the two `#[cfg(feature = "server")]` use-blocks into one grouped
  block; keep the ungated wire-arg imports ungated.
- **`component.rs`** — move `ForgotPasswordPage`/`ResetPasswordPage` from
  `pages/password_reset.rs`; repoint `Topbar` → `crate::topbar::Topbar`;
  preserve the `with_untracked` token read + race comment and the enumeration
  logic; no cfg attributes inside.
- **`mod.rs`** — reduce to wiring: `mod api;`,
  `#[cfg(target_arch = "wasm32")] mod component;`, plus the re-exports
  (`pub use api::{request_password_reset, confirm_password_reset, RequestPasswordReset, ConfirmPasswordReset};`,
  `#[cfg(target_arch = "wasm32")] pub use component::{ForgotPasswordPage, ResetPasswordPage};`).
  ADR-0070-style `//!` doc.
- **Rewire.** `pages/mod.rs`: delete `pub mod password_reset;`, delete
  `pages/password_reset.rs`, change the router import to
  `crate::password_reset::{ForgotPasswordPage, ResetPasswordPage}`.
- **Issue hygiene.** Correct the stale "Blocked by #312" line on #322.

## Out of scope

- Moving `App`/Router out of `pages/mod.rs` to the app entry — that is **#330**.
- Dissolving `pages/ui.rs` / `web::render` — that is **#312**.
- Any change to the reset behavior, wire contract, the `ProfferedPassword` /
  `Password` secret model, the enumeration-avoidance semantics, the mailer, or
  the storage surface — the server fns move verbatim.
- The automated thin-component complexity guard — that is **#306**.

## Verification

`cargo xtask validate --no-e2e` locally (local e2e VM is reaped here; the CI
matrix gates the four `{sqlite,postgres}×{chromium,firefox}` e2e combos). The
load-bearing behavioral checks are the four `password_reset.spec.ts` flows.
`wasm-clippy` (`-p web`) is load-bearing gate surface for the now-wasm-only UI.

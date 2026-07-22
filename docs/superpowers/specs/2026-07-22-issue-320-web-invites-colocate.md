# Spec — #320: converge the `invites` vertical onto the file-level host/wasm split

**Status:** awaiting approval. **Parent:** #303 (umbrella). **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (supersedes ADR-0056),
as amended by #530 (`api.rs` split) and #527 (shared leaves as top-level
`crate::topbar` etc.). No new ADR — a straight application of ADR-0070's
established pattern (`auth` #315 is the template; `email` #318, `media`,
`posts`, `timeline`, `home` already landed under it).

## Problem

The `invites` vertical is split across files by _technology_, not feature, and
its UI still lives in the old `pages/` home:

- `web/src/invites/mod.rs` — the `InviteInfo` wire DTO and the two `#[server]`
  fns (`create_invite`, `list_invites`) inline, with their grouped
  `#[cfg(feature = "server")]` support block. The pre-0070 shape: endpoints in
  `mod.rs` instead of a dedicated `api.rs`.
- `web/src/pages/invites.rs` — the `InvitesPage` `#[component]` + the
  `render_invite_row` view helper, importing `Topbar` from the stale
  `crate::pages::` re-export.

`InvitesPage` also carries a **dead SSR vestige**: an in-component
`#[cfg(feature = "server")]` block that calls
`ResponseOptions::set_status(NOT_FOUND)` when the policy isn't `invite_only`
(`pages/invites.rs:31-42`), plus a doc-comment claiming "Returns 404 (via SSR
response options)". Since #180 removed reactive SSR (ADR-0040/0041), no authed
`#[component]` body ever executes server-side — the projector renders only the
anonymous public surface through the pure `web::render`, and authed routes are
static CSR shells; in the wasm build the block is compiled out entirely. So the
`set_status` is unreachable in every process. It also violates ADR-0070's
zero-cfg-inside-`component.rs` rule, so it cannot move as-is. `list_invites`
already returns `not_found` server-side when the policy isn't invite-only, so
the block was always redundant.

The policy-gating behavior (that `/invites` is not usable on a non-invite-only
site) is currently **unverified** — the e2e suite exercises only the invite-only
happy path.

## Decisions (interview-resolved)

1. **Three-file layout, no `server.rs`, no `status.rs`.** The `#[server]` bodies
   touch only crate/external imports (`require_auth`, `InternalError`,
   `chrono::Utc`, `compose`, `MailSender`, `InviteStorage`, `SiteConfigStorage`,
   `load_registration_policy`, `RegistrationPolicy`, `host::metrics`) — no
   invites-specific host helper — so `server.rs` is omitted and the grouped
   `#[cfg(feature = "server")]` block sits inline in `api.rs` (the
   `media`/`email` pattern). And per ADR-0070 §6 there is **no pure logic to
   extract**: `render_invite_row` is inherently-wasm `view!`-building, and the
   form validation already lives in `crate::forms` / `common::email`. Final
   shape: `mod.rs` / `api.rs` / `component.rs`.
2. **UI moves into wasm-only `component.rs`.** `InvitesPage` +
   `render_invite_row` move to `web/src/invites/component.rs`, declared
   `#[cfg(target_arch = "wasm32")] mod component;` in `invites/mod.rs` with the
   matching gated re-export
   `#[cfg(target_arch = "wasm32")] pub use component::InvitesPage;`. The file
   carries **zero cfg gates inside it**, does not host-compile, and is **not**
   dead-but-exempt — no `cov:ignore` / `#[component]`-exemption reliance.
3. **Delete the dead SSR-vestige `set_status` block; keep the fallback view.**
   Remove the entire `#[cfg(feature = "server")]` `ResponseOptions` block from
   `InvitesPage` (dead since #180; cannot live in `component.rs`). **Keep** the
   client-side `return view! { <p>"Page not found." </p> }` fallback for the
   non-invite-only branch (unchanged visible behavior). Fix the stale
   doc-comment (it no longer 404s via SSR — it renders a client-side fallback)
   and the stale `invite.spec.ts:43` comment.
4. **Add an e2e guard for the policy-gating we're preserving.** A new test in
   `end2end/tests/invite.spec.ts` (the serial `*-admin` project) asserting: with
   `site.registration_policy = open` (non-invite-only), an authenticated
   operator visiting `/invites` sees the `"Page not found."` fallback and **no**
   create form. This locks the behavior kept in Decision 3, which is currently
   untested.
5. **`#[server]` fns + wire types move to `api.rs`, re-exported from a
   wiring-only `mod.rs`.** `create_invite` / `list_invites`, their generated
   wire structs (`CreateInvite`, `ListInvites`), and the `InviteInfo` DTO move
   verbatim to `web/src/invites/api.rs`; `invites/mod.rs` becomes wiring +
   re-exports only, re-exporting those names explicitly (the `InvitesPage`
   `ServerAction::<CreateInvite>` call site and the
   `crate::invites::{list_invites, InviteInfo}` imports depend on them
   resolving), keeping the registrar path `web::invites::<Leaf>` stable.
6. **Repoint `Topbar`** to `crate::topbar::Topbar` (ADR-0070 #527), not the
   stale `crate::pages::Topbar` re-export.
7. **Fix the stale issue text on #320** — same as #318: the body's "Blocked by
   the prereq (#312)" is reversed (native graph: #312 is `blocked_by` every
   vertical, and verticals shipped while #312 is open). Correct it on GitHub.

## Target end state (acceptance floor)

Each criterion is observable so ship-time conformance review can tell delivered
from not:

1. `invites`' UI, `#[server]` fns, and wire types live under `web/src/invites/`;
   **no `web/src/pages/invites.rs` remains**, and its `pub mod invites;` line in
   `web/src/pages/mod.rs` is deleted.
2. `web/src/invites/` contains exactly `mod.rs`, `api.rs`, `component.rs` — and
   **no `server.rs`** and **no `status.rs`** (neither is warranted).
3. `invites/mod.rs` is **wiring only**: `mod` declarations + `pub use`
   re-exports, no items of its own. `target_arch = "wasm32"` appears in the
   vertical **only on the `mod component;` declaration and its paired
   `pub use component::{…}`** — never on an item inside a file.
4. The `#[component]` UI lives in **wasm-only `component.rs`** with **zero cfg
   gates inside the file**; **no `#[cfg(feature = "server")]` block remains
   inside any component** (the `set_status` vestige is deleted). It does not
   host-compile and adds no `cov:ignore` / `#[component]`-exemption reliance.
5. `invites/api.rs` carries the `InviteInfo` DTO + the two `#[server]` fns
   (dual-compiled, ungated) with at most **one grouped
   `#[cfg(feature = "server")]` use-block** for the bodies. Its wire structs
   `CreateInvite`, `ListInvites` and `InviteInfo` are re-exported **by name**
   from `invites/mod.rs`.
6. `InvitesPage` imports `crate::topbar::Topbar`; no `crate::pages::Topbar`
   reference remains in the invites vertical.
7. The router import in `web/src/pages/mod.rs` reads
   `use crate::invites::InvitesPage;`; the
   `<Route path=StaticSegment("invites") view=InvitesPage />` line is otherwise
   unchanged.
8. The non-invite-only branch still renders `<p>"Page not found." </p>`
   client-side; the doc-comment on `InvitesPage` and the `invite.spec.ts`
   comment no longer claim an SSR 404.
9. A new `invite.spec.ts` test asserts that, authenticated as operator with
   `site.registration_policy = open`, `/invites` shows the `"Page not found."`
   fallback and has **no** `input[name="recipient_email"]`. It sets `open` at
   test start (matching the file's self-set pattern); the existing `afterAll`
   open-restore keeps later serial specs unaffected.
10. No fake-value host stub is introduced (ADR-0055 principle).
11. `cargo xtask validate --no-e2e` green locally (static + `wasm-clippy` +
    coverage); the full e2e matrix green in CI (local e2e VM is reaped here).
    Because the components are now wasm-only, **`wasm-clippy` is load-bearing
    gate surface** for this vertical's UI type-checking.
12. `end2end/tests/invite.spec.ts` stays green — the existing "invite link
    registration completes end-to-end" and "invite-only /register …" tests, plus
    the new policy-gating guard. Load-bearing selectors/strings preserved:
    `input[name="recipient_email"]`, `input[name="expires_in_hours"]`, submit,
    and `"Invitation emailed to"`.
13. #320's stale "Blocked by #312" line is corrected on GitHub.

## Shape of the work

- **`api.rs`** — move `InviteInfo`, the two `#[server]` fns, and the
  `#[cfg(feature = "server")]` use-block from today's `invites/mod.rs` into a
  new `invites/api.rs`, unchanged.
- **`component.rs`** — move `InvitesPage` + `render_invite_row` from
  `pages/invites.rs`; repoint `Topbar` → `crate::topbar::Topbar`; **delete the
  `#[cfg(feature = "server")]` `set_status` block** (keep the
  `"Page not found."` return); refresh the doc-comment; no cfg attributes
  inside.
- **`mod.rs`** — reduce to wiring: `mod api;`,
  `#[cfg(target_arch = "wasm32")] mod component;`, plus the re-exports
  (`pub use api::{create_invite, list_invites, CreateInvite, ListInvites, InviteInfo};`,
  `#[cfg(target_arch = "wasm32")] pub use component::InvitesPage;`).
  ADR-0070-style `//!` doc.
- **Rewire.** `pages/mod.rs`: delete `pub mod invites;`, delete
  `pages/invites.rs`, change the router import to `crate::invites::InvitesPage`.
- **e2e.** Add the policy-gating guard test to `invite.spec.ts`; update the
  stale line-43 comment.
- **Issue hygiene.** Correct the stale "Blocked by #312" line on #320.

## Out of scope

- Moving `App`/Router out of `pages/mod.rs` to the app entry — that is **#330**.
- Dissolving `pages/ui.rs` / `web::render` — that is **#312**.
- Any change to the invite creation/listing behavior, wire contract, the
  `InviteCode` secrecy model (#400), the mailer, or the `InviteStorage` surface
  — the server fns and `InviteInfo` DTO move verbatim.
- The automated thin-component complexity guard — that is **#306**.

## Verification

`cargo xtask validate --no-e2e` locally (local e2e VM is reaped here; the CI
matrix gates the four `{sqlite,postgres}×{chromium,firefox}` e2e combos). The
load-bearing behavioral checks are the three `invite.spec.ts` flows (the two
existing + the new policy-gating guard). `wasm-clippy` (`-p web`) is
load-bearing gate surface for the now-wasm-only UI.

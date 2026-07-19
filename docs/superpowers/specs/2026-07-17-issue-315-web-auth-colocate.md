# Spec — #315: converge the `auth` vertical onto the file-level host/wasm split

**Status:** reconciled to ADR-0070; awaiting re-approval. **Parent:** #303
(umbrella). **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (supersedes ADR-0056),
with `docs/web-style-guide.md` §8 as the layout template. **Rework plan of
record:** the maintainer's 2026-07-18 review comment on #315.

> **Reconciliation note (2026-07-18).** This spec was originally written against
> **ADR-0056** (co-located verticals split _by cargo feature, never
> `target_arch`_; `#[component]` UI host-compiles as dead-but-exempt). ADR-0070
> **supersedes ADR-0056**: each vertical now splits host and wasm at the
> **file** level — the `#[component]` UI moves into a wasm-only `component.rs`
> (`#[cfg(target_arch = "wasm32")] mod component;`), which never host-compiles
> and calls `client::`/`web-sys` directly. The convergence goal (UI +
> `#[server]` fns + wire types in one vertical, out of `pages/`) is unchanged;
> only the _gating_ is reversed. The maintainer's #315 review comment
> establishes that **~75–85% of the already-landed PR #508 diff survives
> verbatim**; this spec is reconciled to keep that work and redo only the
> co-location, drop the marker commit, and land on the new base.

> **Amendment note (2026-07-18, #530 / PR #531).** ADR-0070 was amended: a
> vertical's `#[server]` endpoints and their wire DTOs move out of `mod.rs` into
> a dedicated **`api.rs`**; `mod.rs` becomes **wiring only** — module
> declarations (`mod api;`, gated `mod server;`/`mod component;`) and re-exports
> (`pub use api::{…};`, gated `pub use component::{…};`). The re-exports keep
> external call-site and server-fn-registrar paths (`web::<vertical>::<Leaf>`)
> stable. `target_arch = "wasm32"` appears only on module-wiring lines (`mod`
> declarations and their paired `pub use`), never inside a leaf file. So the
> four-file layout is `mod.rs` / `api.rs` / `server.rs` / `component.rs`.
> Anywhere below that says "`#[server]` fns live in `mod.rs`" now means
> **`api.rs`**, re-exported from `mod.rs`.

## Problem

The `auth` vertical is split across files by _technology_, not feature:

- `web/src/pages/auth.rs` — the three `#[component]`s (`RegisterPage`,
  `LoginPage`, `LogoutPage`) + the `invite_link_required` helper. The old
  `pages/` home.
- `web/src/auth/mod.rs` — the `#[server]` fns (`get_registration_policy`,
  `current_user`, `register`, `login`, `logout`) and the generated action
  structs (`Register`/`Login`/`Logout`) the components consume.
- `web/src/auth/server.rs` — SSR-only internals (extractors, cookie helpers,
  `require_auth`, host tests). Correctly server-gated — already the ADR-0070
  `server.rs`.
- `web/src/auth/marker.rs` — the ADR-0044 advisory marker **pure codec**
  (`encode_marker`/`decode_marker` + `MARKER_KEY`), host-tested and cfg-free.
- `web/src/auth/marker_storage.rs` — the wasm-only `localStorage` binding
  (`get`/`set`/`remove`) over `client::storage`,
  `#[cfg(target_arch = "wasm32")]`. **Landed by #514 / PR #525** — auth's marker
  glue is already in its ADR-0070 shape.

The UI and the server fns of one feature live in separate homes — the split
ADR-0070 §Decision eliminates. auth's UI also still imports `Topbar` from the
`crate::pages::` re-export rather than the `crate::ui` home.

## Decisions (interview-resolved, reconciled to ADR-0070)

1. **The UI moves into wasm-only `component.rs` files (ADR-0070), _not_ ungated
   into `mod.rs`.** `LoginPage` and `LogoutPage` go into
   `web/src/auth/component.rs`, declared
   `#[cfg(target_arch = "wasm32")] mod component;` in `auth/mod.rs` with the
   matching gated re-export
   `#[cfg(target_arch = "wasm32")] pub use component::{LoginPage, LogoutPage};`
   (`InviteLinkRequired` lands with registration — Decision 4). The file carries
   **zero cfg gates inside it** and calls browser code directly. Auth ends with
   the ADR-0070 three-file shape (`mod.rs` / `server.rs` / `component.rs`) plus
   the marker pair.
   - _This reverses the original spec's Decision 1_ ("drop the marker's
     `target_arch` gate; host-compile web-sys; `cov:ignore` the wrappers").
     Under ADR-0070 the components don't host-compile, so there is **nothing to
     `cov:ignore`** and no `#[component]`-exemption calculus — that machinery is
     deleted, not relocated. The marker's browser binding is already the
     wasm-gated `marker_storage.rs`; components call
     `marker_storage::{get, set, remove}`.
2. **Scope = convergence + API tightening.** Beyond the co-location, tighten the
   auth surface where the move surfaces a clarity win (enumerated in the plan).
   The file-level split and rewiring are the invariant floor.
3. **Typed password wire arg via a `ProfferedPassword` inbound-secret twin (KEPT
   — direction-neutral, already landed).** The secret `Password`
   (`#[str_newtype(secret)]`) can't be a `#[server]` wire arg (no serde by
   design). `common::password::ProfferedPassword`
   (`#[str_newtype(secret, serde)]`, ADR-0063's inbound-secret variant) — the
   same `Proffered*` twin pattern #400 established for invite codes — is taken
   on the `register`/`login`/`confirm_password_reset` wire and converted to
   `Password` at the boundary, shipping the generalized `proffered-secret` xtask
   gate. This supersedes #410's "password stays `String`"; no `Password` change,
   so the secret invariant is untouched. This is a real serde/secret constraint
   that **survives any layout direction** and is retained verbatim from the
   landed work.
4. **Split registration out of auth (KEPT — already landed).** `register` +
   `get_registration_policy` + `RegisterPage` + the invite guidance are
   account-provisioning, not authentication, and live in a
   `web/src/registration/` vertical. Under ADR-0070, `RegisterPage` +
   `InviteLinkRequired` go into `web/src/registration/component.rs` (same wasm
   gate); registration logs the new user in via auth's `pub(crate)`
   `set_session_cookie`.
5. **`tracing` is an unconditional `web` dependency (KEPT — already landed,
   supersedes ADR-0013's cfg-split half).** `#[tracing::instrument]` is written
   plainly (no-op on the wasm client) instead of a per-fn
   `#[cfg_attr(feature = "server", …)]`. This _aligns_ with ADR-0070's "one
   support gate" minimalism and is retained.

## Target end state (acceptance floor — reconciled to ADR-0070)

1. `auth`'s UI, `#[server]` fns, and wire types live under `web/src/auth/` (and
   registration's under `web/src/registration/`); **no `web/src/pages/auth.rs`
   remains** and its `pub mod auth;` at `pages/mod.rs` is deleted.
2. The `#[component]` UI lives in **wasm-only `component.rs`** files, declared
   `#[cfg(target_arch = "wasm32")] mod component;` on the `mod` line only —
   **zero cfg gates inside the file**; the components **do not host-compile**
   and are **not** dead-but-exempt. **No `cov:ignore` and no
   `#[component]`-exemption reliance is added** to satisfy host compilation of
   UI.
3. `target_arch = "wasm32"` appears in the vertical **only on `mod`
   declarations** (`mod component;`, `mod marker_storage;`), never on an item
   inside a file (ADR-0070 §2).
4. The client/server split of the `#[server]` bodies is expressed only via
   `feature = "server"` + the `#[server]` macro; `auth/server.rs` stays the
   `#[cfg(feature = "server")] mod server` host-only support.
5. UI imports the shared `Topbar` from `crate::ui::Topbar` (the wasm-gated
   shared home), not `crate::pages::Topbar`.
6. The router import in `pages/mod.rs` reads
   `use crate::{auth::{LoginPage, LogoutPage}, registration::RegisterPage};`,
   backed by a gated re-export in each vertical's `mod.rs`
   (`#[cfg(target_arch = "wasm32")] pub use component::{…};`) — a bare private
   `mod component;` leaves the components unreachable. The gated `mod` + gated
   `pub use` pair is the template form later verticals copy; the `<Route>` lines
   are otherwise unchanged.
7. ADR-0044 marker behavior is preserved exactly: `marker_storage::set` on
   register/login success, `marker_storage::remove` on logout success;
   `marker.rs` stays the pure host-tested codec, in sync with
   `render::PREPAINT_SCRIPT`. **No re-implementation of the marker glue** — it
   is consumed from #525's `marker_storage.rs`.
8. Pure, host-testable logic (validation, `Field<T>`-style signal/form state,
   codecs) stays in **ungated, host-tested** files, extracted _before_ any gate
   (ADR-0070 §6); no fake-value host stub is introduced (ADR-0055 principle).
9. `cargo xtask validate` green, including the auth e2e flows (register, login,
   logout, and the invite-only register-guidance path).

## Shape of the work (reconciled)

- **Rebase onto `origin/main`.** #525 (marker glue) and ADR-0070 are already in
  `main`. The co-location commit is the only real conflict (see the plan's
  rebase recipe); the ~75–85% survives.
- **Re-home the UI into `component.rs` (redo the co-location commit).**
  `LoginPage`/`LogoutPage` → `web/src/auth/component.rs`; `RegisterPage` +
  `InviteLinkRequired` + invite guidance → `web/src/registration/component.rs`.
  Bodies move essentially unchanged — cut-paste-and-gate, not a redesign.
- **Drop the marker commit.** No un-gating of `marker.rs`, no `cov:ignore`
  reasoning, no `#[component]`-exempt calculus. Repoint component call sites at
  `marker_storage::{get, set, remove}`.
- **Rewire.** `Topbar` → `crate::ui::Topbar`; router import → `crate::auth::…` /
  `crate::registration::…`.
- **Module docs.** ADR-0070-style `//!` docs on `auth/mod.rs`,
  `registration/mod.rs`, and the new `component.rs` files.
- **Keep verbatim:** `ProfferedPassword` + `validate_password_shape` + the
  `proffered-secret` gate; the registration/auth split; the
  tracing-unconditional commit; `auth/server.rs`; the server-test flips (assert
  non-`Ok`, not the message).

## Out of scope

- Moving `App`/Router out of `pages/mod.rs` to the app entry — that is **#330**.
- Dissolving `pages/ui.rs` / `web::render` — that is **#312**. This issue only
  stops _importing_ auth's UI from `pages/`; it does not remove the
  `pages/mod.rs` re-export shim.
- Any change to `auth/server.rs`'s SSR internals beyond what co-location forces.
- Re-implementing or altering the `marker.rs` / `marker_storage.rs` pair (#514 /
  #525 own it).

## Verification

`cargo xtask validate` (static + wasm-clippy + coverage + full e2e matrix). The
load-bearing behavioral checks are the auth e2e flows: register (open +
invite-only guidance + invite-code register), login, logout — each must still
drive the localStorage marker set/clear correctly (ADR-0044). Because the
components are now wasm-only, **`wasm-clippy` is load-bearing gate surface** for
this vertical's UI type-checking (ADR-0070 §Consequences), not just host clippy.

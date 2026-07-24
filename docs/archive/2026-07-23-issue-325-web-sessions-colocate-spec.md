# Spec — #325: converge the `sessions` vertical + `SessionLabel` newtype

**Status:** awaiting approval. **Parent:** #303 (umbrella, milestone #11).
**Decision records:** `docs/adr/0070-web-vertical-wasm-only-component-files.md`
(file-level host/wasm split, amended #530 — `#[server]` fns + wire DTOs in
`api.rs`, `mod.rs` wiring-only), `docs/adr/0065-*` (typed-wire-arg /
client-validated direct-bind forms), `docs/adr/0063-*` (the string-newtype
trailer). Layout template: the shipped `profile`/`auth`/`media` verticals;
newtype template: `common::display_name::DisplayName`. **No new ADR** — this
applies existing decisions.

> **Security-adjacent (full ship review regardless of size).** This vertical
> manages app-password tokens (`RawToken`), session hashes (`TokenHash`), and
> `revoke_session`. The `revoke_session` ownership check (a caller may only
> revoke a session in _their own_ list) is load-bearing and must be preserved
> verbatim.

## Problem

The `sessions` vertical is split by _technology_ and its `mod.rs` is not
wiring-only:

- `web/src/sessions/mod.rs` — holds the `SessionInfo` + `AppPassword` wire DTOs,
  the grouped `#[cfg(feature = "server")]` use-block, and the three `#[server]`
  fns (`list_sessions`, `create_app_password`, `revoke_session`). Under ADR-0070
  (amended #530) this belongs in `api.rs`.
- `web/src/pages/sessions.rs` — the `SessionsPage` `#[component]`; imports
  `Topbar` from the `crate::pages::` shim.

Separately, the vertical still uses the pre-ADR-0065 patterns: two
`<ActionForm>`s (create-app-password with a stringly `<input name="label">`,
revoke with a hidden `<input name="token_hash">`), and the app-password **label
is a bare `String`** threaded through the web wire _and_ storage
(`create_session(&str)`, `SessionRecord.label: String`) — a domain concept (a
validated, non-empty, bounded human label) modelled as a primitive.

## Decisions (interview-resolved)

1. **Three-file vertical.** `mod.rs` (wiring) / `api.rs` (DTOs + `#[server]`
   fns + server use-block) / `component.rs` (wasm-only `SessionsPage`). **No
   `server.rs`** (no host-only support) and **no extraction file** (the
   component is view-only).
2. **Modernize both `<ActionForm>`s to ADR-0065 typed direct-bind.**
   - **revoke:** replace the hidden-input form with a plain `type="button"` that
     dispatches `RevokeSession { token_hash }` (the hash is a known value per
     row, `TokenHash` is `Clone`).
   - **create:** a `Field<SessionLabel>`-bound `<input>` + a plain
     `type="button"` that dispatches `CreateAppPassword { label }`,
     disabled-until-valid — mirroring the profile display-name form.
3. **Introduce `common::session_label::SessionLabel`** — a
   `#[derive(StrNewtype)]` validated label (trim, non-empty, ≤
   `MAX_SESSION_LABEL_CHARS = 255`), modelled exactly on `DisplayName` (ADR-0063
   trailer: `Display`/`AsRef<str>`/`Deref`/owned conversions/`PartialEq<str>` +
   the validating serde and sqlx bridges). **255 is chosen for `DisplayName`
   parity and to keep the read-side decode (Decision 4) safe** — a realistic
   device/app label is far shorter.
4. **Thread `SessionLabel` through the wire _and_ storage (both write and
   read).**
   - Wire (write): `create_app_password(label: SessionLabel)` — a typed wire arg
     (ADR-0065); the server's manual `trim`/non-empty check is deleted (the
     newtype guarantees it at decode). `AppPassword.label: SessionLabel` (echoed
     back).
   - Wire (read): `SessionInfo.label: SessionLabel`.
   - Storage (write):
     `SessionStorage::create_session(user_id, label: &SessionLabel)`.
   - Storage (read): `SessionRecord.label: SessionLabel`.
   - **All four `create_session` callers are retyped (compiler-forced sweep).**
     Besides `create_app_password`, `create_session` is called by three other
     production sites, which must now construct a `SessionLabel`:
     - **login** (`web/src/auth/api.rs`): the User-Agent-derived label (fallback
       `"Unknown device"`, already truncated to 200 chars) becomes a
       `SessionLabel` — parse the derived string, falling back to a
       `"Unknown device"` `SessionLabel` if it is empty/whitespace (which the
       current code does not guard, since the old path never validated).
     - **registration** (`web/src/registration/api.rs`): the literal
       `"Sign-up session"` is parsed to a `SessionLabel` (known-valid).
     - **CLI** (`server/src/commands.rs`, the `app-password create` command):
       the **user-supplied** `--label` arg becomes a `SessionLabel` — so the CLI
       path now gets the same non-empty/≤255 validation as the web wire (a small
       bonus of the sweep, previously unvalidated).
   - **Read side is decoded leniently (revised after the standards review).**
     `SessionRecord.label` is still typed `SessionLabel`, but the `label` column
     is decoded as a plain `String` and repaired into a `SessionLabel` via
     `SessionLabel::from_lossy` in `storage::helpers::session_record_from_row` —
     **not** the validating sqlx `Decode`. A label is a best-effort _display_
     value, and the old CLI minter (`server/src/commands.rs`) applied no length
     or non-empty check, so a pre-existing empty/over-long row must not break
     the whole `list_sessions` query. `from_lossy` trims, truncates, and
     defaults such a row on read, so **read-decode never fails** and no data
     migration is needed. The validating `FromStr`/serde path still guards the
     _write_ side (the wire arg + CLI). `MAX_SESSION_LABEL_CHARS` is therefore
     no longer load-bearing for row survival; login keeps its own 200-char
     device-name cap as product behavior, with `from_lossy` the infallible type
     construction.

## Target end state (observable criteria)

1. **Co-located, `pages/` home gone.** `sessions`' UI + `#[server]` fns + wire
   types live under `web/src/sessions/`; **no `web/src/pages/sessions.rs`**, and
   `pub mod sessions;` at `pages/mod.rs:1` is deleted.
2. **`mod.rs` wiring only** — `//!` doc, gated `mod` decls, re-exports; no DTO,
   `#[server]` fn, or use-block of its own.
3. **`api.rs`** holds `SessionInfo`, `AppPassword`, the three `#[server]` fns,
   and the single grouped `#[cfg(feature = "server")]` use-block
   (dual-compiled).
4. **`component.rs`** holds `SessionsPage`, wasm-only
   (`#[cfg(target_arch = "wasm32")] mod component;` on the `mod` line only, zero
   cfgs inside); no `cov:ignore` / `#[component]`-exemption added; `Topbar` from
   `crate::topbar`.
5. **`target_arch`** appears only on the `mod component;` + paired `pub use`
   lines.
6. **Router** import reads `use crate::sessions::SessionsPage;`, backed by the
   gated `pub use component::SessionsPage;`; the `/sessions` `<Route>` is
   otherwise unchanged. (No external code consumes `sessions::*` — the only
   reference is a doc comment in `auth/marker.rs`.)
7. **`SessionLabel` exists** in `common` with the `DisplayName`-parity API and
   unit tests (parse/trim/non-empty/cap/`Display`/serde round-trip + reject).
8. **Wire + storage typed (Decision 4):** `create_app_password`,
   `AppPassword.label`, `SessionInfo.label`, `create_session`, and
   `SessionRecord.label` all use `SessionLabel`; **all four `create_session`
   callers compile** — `create_app_password` (typed wire arg), login
   (`auth/api.rs`, UA-derived `SessionLabel` with an `"Unknown device"`
   fallback), registration (`registration/api.rs`, `"Sign-up session"` parsed),
   and the CLI `app-password create` (`server/src/commands.rs`, user-supplied
   `--label`); the server's manual label validation is gone; `revoke_session`'s
   ownership check is unchanged. 8a. **Session security + label tests stay green
   (and gain the cap case).** `server/tests/web/web_sessions.rs` **already**
   pins the security invariant —
   `revoke_session_rejects_session_belonging_to_another_user` (Alice can't
   revoke Bob's session; dual-backend) — and
   `create_app_password_rejects_blank_label`. Both must stay green through the
   refactor. Because the server-side manual check is deleted (Decision 4), the
   blank-label rejection now happens at **arg-decode**;
   `create_app_password_rejects_blank_label`'s expected status is re-verified
   and adjusted to the decode-rejection status if it shifts (500 → likely 400).
   **Add** `create_app_password_rejects_overlong_label` (a
   `> MAX_SESSION_LABEL_CHARS` label is rejected) — new coverage the cap makes
   possible.
9. **Forms modernized (Decision 2):** no `<ActionForm>` and no
   `<input name="label">` / `<input name="token_hash">` submit path remain;
   create uses a `Field<SessionLabel>` input hooked by a stable id,
   disabled-until-valid; revoke uses a `type="button"` dispatch.
10. **e2e green + honest.** `end2end/tests/atompub.spec.ts` (which mints an app
    password via the Sessions UI and uses the token over AtomPub Basic auth)
    passes with its selectors updated to the direct-bind control (the
    `type="submit"` locator becomes the button-by-text / id hook); the
    app-password token still surfaces in `.j-app-password-token code` and the
    session appears under its label.
11. **Gate green.** `cargo xtask validate` — static + clippy + wasm-clippy +
    dual-backend coverage/tests + e2e — including a storage round-trip test for
    the typed label and a web wire test that `create_app_password` rejects an
    empty / over-long `label` at decode.

## Shape of the work

- **`common`:** add `session_label.rs` (`SessionLabel` + `InvalidSessionLabel` +
  `MAX_SESSION_LABEL_CHARS` + tests), `pub mod session_label;` in `lib.rs`.
- **`storage`:** `create_session` takes `&SessionLabel`;
  `SessionRecord.label: SessionLabel` (and its `SessionRow` decode via
  `session_record_from_row`); update the storage tests + any fixtures that build
  a `SessionRecord`/pass a label. Follow backend-parity.
- **`server/tests/web/web_sessions.rs`:** the existing security/label tests stay
  green (criterion 8a); adjust `create_app_password_rejects_blank_label`'s
  status if decode-rejection shifts it; add
  `create_app_password_rejects_overlong_label`.
- **Other `create_session` callers (compiler-forced):** `web/src/auth/api.rs`
  (login — derive a `SessionLabel` from the truncated UA, `"Unknown device"`
  fallback) and `web/src/registration/api.rs` (parse `"Sign-up session"`).
- **`web/src/sessions/api.rs`:** move + retype the DTOs and `#[server]` fns;
  drop the manual label check; add the wire test.
- **`web/src/sessions/component.rs`:** move `SessionsPage`; modernize both
  forms; `Topbar` → `crate::topbar`; endpoints via `super::api`.
- **`web/src/sessions/mod.rs`:** `//!` doc + wiring + re-exports.
- **Rewire `pages/mod.rs`; delete `pages/sessions.rs`.**
- **e2e:** update `atompub.spec.ts` sessions-UI selectors + any stale comment.

## Out of scope

- #330 (App/Router move), #312 (dissolve `pages/ui.rs` / the `crate::pages::`
  shim itself).
- Any change to session **auth** semantics (cookie/token minting,
  `authenticate`) beyond retyping the `label` field.
- Typing other `SessionRecord` fields or unrelated storage labels.

## Verification

`cargo xtask validate`. Load-bearing behavioral checks: the AtomPub e2e (mint an
app password via the modernized Sessions form → use it for HTTP Basic
publishing), the storage dual-backend label round-trip, the `revoke_session`
ownership regression test (criterion 8a), and the web wire test proving
`create_app_password` rejects empty/over-long labels at decode. `wasm-clippy` is
load-bearing for the now-wasm-only `SessionsPage`. Because the label validation
now lives in the `SessionLabel` chokepoint, the deleted server-side check cannot
silently weaken it — the wire test pins the endpoint's rejection.

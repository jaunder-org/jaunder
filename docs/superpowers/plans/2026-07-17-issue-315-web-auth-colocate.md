# Plan — #315: converge the `auth` vertical onto the file-level host/wasm split

**Spec:** `docs/superpowers/specs/2026-07-17-issue-315-web-auth-colocate.md`
(problem/why/decisions live there — this plan is "how"). **Reconciled to
ADR-0070** (2026-07-18); the rework recipe of record is the maintainer's
2026-07-18 review comment on #315. **For agentic workers:** drive with
`jaunder-iterate`; tick checkboxes in real time.

## Reconciliation summary — what changed vs the original plan

The original plan (all six tasks marked done, PR #508 open) landed against
**ADR-0056**. ADR-0070 supersedes it: UI moves into **wasm-only
`component.rs`**, not ungated into `mod.rs`. Per the maintainer's #315 review,
**~75–85% of the landed diff survives verbatim**; this is a rebase whose
conflict resolutions _are_ the re-homing.

> **Amendment (2026-07-18, #530 / PR #531).** ADR-0070 was amended after this
> plan was written: `#[server]` endpoints + wire DTOs move from `mod.rs` into a
> dedicated **`api.rs`**, and `mod.rs` becomes **wiring only** (module
> declarations + re-exports). The final layout is the four-file `mod.rs` /
> `api.rs` / `server.rs` / `component.rs`. This was applied as a follow-on:
> `auth/api.rs` holds `current_user`/`login`/`logout`, `registration/api.rs`
> holds `get_registration_policy`/`register`, and each `mod.rs` re-exports them
> (`pub use api::{…}`) so registrar and external call sites keep the stable
> `web::<vertical>::<Leaf>` paths. Where the tasks below say "`#[server]` fns in
> `mod.rs`", read **`api.rs`**.

- **Keep verbatim (already landed, direction-neutral — replay clean on
  rebase):** `ProfferedPassword` + `validate_password_shape`
  (`common/src/password.rs`) + the generalized `proffered-secret` xtask gate;
  the registration/auth vertical split; the tracing-unconditional /
  ADR-0013-supersession commit; `auth/server.rs` untouched; the server-test
  flips (assert non-`Ok`, not the message) and all `#[server]` co-location.
- **Redo (the co-location commit `dfa1e9f5` only):** `LoginPage` / `LogoutPage`
  / `InviteLinkRequired` → `web/src/auth/component.rs`
  (`#[cfg(target_arch = "wasm32")] mod component;`); `RegisterPage` + invite
  guidance → `web/src/registration/component.rs`. Bodies move essentially
  unchanged — **cut-paste-and-gate, not a redesign**.
- **Drop entirely:** the marker un-gating (un-gate `read`/`set`/`clear` +
  `cov:ignore` "host-unreachable" reasoning) and every `cov:ignore` /
  `#[component]`-exempt calculus added to keep UI host-compiling. PR #525 (#514)
  already owns the marker glue: `marker.rs` = pure cfg-free codec, browser
  binding in wasm-gated `marker_storage.rs` over `client::storage`. Components
  call `marker_storage::{get, set, remove}`.

## Base facts (verified against `origin/main`, 2026-07-18)

- `web/src/auth/marker_storage.rs` exists in `main` (from #525), wasm-gated,
  exposing `get() -> Option<String>`, `set(username: &str)`, `remove()`. **The
  logout call site uses `remove`, not `clear`.**
- `web/src/auth/mod.rs` in `main` already declares `pub mod marker;` (ungated)
  and `#[cfg(target_arch = "wasm32")] pub mod marker_storage;`, and holds the
  `#[server]` fns. Only the UI needs a home.
- **No vertical in `main` has a `component.rs` yet** — this rework establishes
  the first instance of the ADR-0070 layout. There is no in-repo template;
  follow ADR-0070 §Decision + `docs/web-style-guide.md` §8.
- `web/src/ui/topbar.rs` exists; `Topbar` comes from `crate::ui::Topbar`.
- `web/src/pages/auth.rs` still exists in `main` (the #525-edited copy, 3
  `marker_storage` call sites). #508 deletes it → **modify/delete conflict** on
  rebase; resolve by keeping the delete and harvesting behavior into the new
  `component.rs` files.

## Global constraints

- Rust; edits under `web/src/`, `common/src/` (kept), `xtask/` (kept). No
  `Co-Authored-By` trailer.
- Per-task check: `cargo xtask check --no-test` while iterating (host static +
  clippy + **wasm-clippy** — load-bearing now that components are wasm-only);
  the full `cargo xtask check` (Nix coverage) before any commit (pre-commit hook
  runs it).
- `leptosfmt` runs in the gate; keep intent comments **outside** `view!` (it
  relocates them) and beware it mangling generic component tags (memory).
- Behavioral proof is e2e: register (open / invite-only guidance / invite-code),
  login, logout — each drives the ADR-0044 marker via `marker_storage`.
  `cargo xtask validate` before ship.
- **Serialize edit → gate → commit** (Nix builds the working tree mid-commit;
  concurrent edits corrupt coverage — memory `no_edit_during_gated_commit`).

---

## Task 0 — Commit the reconciled spec + plan

This spec/plan pair currently exists only as **staged, uncommitted** moves in
the worktree (un-archived from `docs/archive/` back into `docs/superpowers/`),
and Task 1 opens with `git rebase`, which **refuses a dirty tree**. Commit them
first as a docs-only commit (the pre-commit gate is cheap on docs), e.g.
`docs: reconcile #315 spec + plan to ADR-0070`. Ship re-archives them later as
usual.

**Check:** `git status --porcelain` → clean.

---

## Task 1 — Rebase `#508` onto `origin/main`, re-homing the UI into `component.rs`

The rebase stops on the co-location commit `dfa1e9f5` with four conflicts:
`auth/marker.rs`, `auth/mod.rs`, `pages/auth.rs` (modify/delete),
`pages/mod.rs`. A backup tag `pre-rebase-508` is already set at the pre-rebase
HEAD.

**Conflict resolution recipe (this _is_ the redo — resolve toward ADR-0070, not
the old commit's tree):**

- `web/src/auth/marker.rs` — **take `main`'s version** (pure codec; drop the
  branch's un-gating). Subtractive resolution.
- `web/src/pages/auth.rs` — **keep the delete** (`git rm`); its behavior (incl.
  #525's `marker_storage` call sites) is re-homed into the `component.rs` files
  below, not left in `pages/`.
- `web/src/auth/mod.rs` — keep `main`'s `#[server]` fns, the `pub mod marker;` +
  `#[cfg(target_arch = "wasm32")] pub mod marker_storage;` decls, and the kept
  ProfferedPassword wire changes. **Add**
  `#[cfg(target_arch = "wasm32")] mod component;` **and the matching gated
  re-export**
  `#[cfg(target_arch = "wasm32")] pub use component::{LoginPage, LogoutPage};` —
  a bare private `mod` leaves the components unreachable from `pages/mod.rs`.
  This gated `mod` + gated `pub use` pair is the template form later verticals
  copy. Do **not** append components into `mod.rs`; do **not** re-gate the
  marker.
- **Create `web/src/auth/component.rs`** (new, wasm-only, **zero inner cfgs**):
  `LoginPage`, `LogoutPage`, and `InviteLinkRequired` (already a `#[component]`
  from the landed T1). Imports: `crate::ui::Topbar` (**not** `crate::pages`),
  `crate::error::WebError`, `crate::forms::{Field, ValidatedInput}`,
  `common::password::ProfferedPassword`, `leptos::prelude::*`, and
  `super::{Login, Logout, marker_storage}` etc. Replace any
  `marker::{set,clear}` with `marker_storage::set(..)` /
  `marker_storage::remove()`; reads use `marker_storage::get()`. Drop
  `#[client_only]` and any `cov:ignore`.
- `web/src/pages/mod.rs` — delete `pub mod auth;`; router import →
  `use crate::{auth::{LoginPage, LogoutPage}, registration::RegisterPage};` (the
  `RegisterPage` home settles after Task 2's replay). `<Route>` lines unchanged.

Continue the rebase. The kept commits (ProfferedPassword, xtask gate, wire,
tracing) should replay clean in substance — but expect additional **trivial**
conflict stops on the tracing commit (`f3b46433`): it touches `web/Cargo.toml`
(edited by #525 on `main`), `auth/mod.rs` (rewritten by this task's resolution),
and `docs/README.md` (edited by #529's ADR-table sync). Those are routine
resolutions, not plan failure.

**Check:** `cargo xtask check --no-test` (host static + clippy + wasm-clippy)
after resolving. `rg 'target_arch' web/src/auth` → hits **only** on
`mod marker_storage;` and `mod component;` declaration lines.

**Result:** a rebased branch whose auth UI is wasm-only in `component.rs`. (The
commit is the amended `dfa1e9f5`; keep its `(#315)` subject or reword to
`feat(web): co-locate the auth UI into wasm-only auth/component.rs (#315)`.)

---

## Task 2 — Replay the registration extraction into `registration/component.rs`

The registration-extraction commit (`eeb952dc`) replays after co-location. It
originally moved `RegisterPage` + `get_registration_policy` + `register` into
`web/src/registration/mod.rs` (ungated UI). **Adjust its resolution** so:

- `RegisterPage` + `InviteLinkRequired` (if it belongs with registration's
  invite guidance) land in **`web/src/registration/component.rs`**
  (`#[cfg(target_arch = "wasm32")] mod component;` plus the gated re-export
  `#[cfg(target_arch = "wasm32")] pub use component::RegisterPage;` in
  `registration/mod.rs` — same template form as auth), not ungated in `mod.rs`.
- `registration/mod.rs` keeps the ungated wire types + `register` /
  `get_registration_policy` `#[server]` fns and the grouped
  `#[cfg(feature = "server")]` support gate; it logs the new user in via auth's
  `pub(crate) set_session_cookie`.
- Marker call sites → `marker_storage::set` on register success.

Decide `InviteLinkRequired`'s home: it belongs with `RegisterPage` (invite
guidance is a registration concern) → `registration/component.rs`. If
`LoginPage` does not reference it, it need not stay in `auth/component.rs`.

**Check:** `cargo xtask check --no-test`;
`rg 'target_arch' web/src/registration` → only the `mod component;` decl line.

---

## Task 3 — Scrub old-doctrine residue; module docs

- Confirm **no** `cov:ignore` and **no** `#[client_only]` / `#[component]`
  coverage-exemption reliance remain in the two verticals (all deleted with the
  marker commit — verify none re-appeared):
  `rg 'cov:ignore|client_only' web/src/auth web/src/registration` → no hits.
- ADR-0070-style `//!` module docs on `auth/mod.rs`, `registration/mod.rs`, and
  the two `component.rs` files (feature summary; a short Authorization/marker
  note on auth; `component.rs` doc notes it is wasm-only UI calling browser
  primitives directly).
- Update `auth/mod.rs`'s marker doc comment to name the real binding API
  (`get`/`set`/`remove`) if it still says `read`/`set`/`clear`.

**Check:** `cargo xtask check --no-test` (doc-only; `missing_docs`-clean).

---

## Task 4 — Verify the KEPT work survived the rebase

No new edits; confirm the landed, direction-neutral work is intact on the new
base:

- `common/src/password.rs` — `ProfferedPassword` + `validate_password_shape`
  present; `TryFrom<ProfferedPassword> for Password` present.
- The `proffered-secret` xtask gate present and **bites** (run it; memory
  `typesafety_needs_enforcement_gate`).
- `register` / `login` / `confirm_password_reset` take `ProfferedPassword` on
  the wire; server tests assert non-`Ok` (not a message).
- The tracing-unconditional commit intact; **zero**
  `#[cfg_attr(feature = "server", tracing::instrument…)]` remain in the touched
  verticals.

**Check:** `cargo xtask check` (full, incl. coverage) green. Confirm coverage is
clean **without** the retired `#[component]`-exemption reliance — component
lines now leave the host denominator entirely (ADR-0070 §Consequences), so an
aggregate percentage shift is re-scoping, not regression (the gate is stateless
— there is no baseline to sign off). A genuine coverage failure here means a
**pure-logic** line lost its host coverage — fix the code (restore or extract
its host test); don't seek approval.

---

## Final verification (before ship)

`cargo xtask validate` — static + **wasm-clippy** + coverage + full e2e matrix
(`{sqlite,postgres}×{chromium,firefox}`). Confirm the auth e2e flows pass:
register (open / invite-only guidance / invite-code), login, logout — each
driving the localStorage marker set/remove via `marker_storage` (ADR-0044). Then
`jaunder-ship`.

## Risks / notes

- **The rebase resolution is the rework** — there is no separate "landing
  rebase" that avoids re-homing. Resolve `dfa1e9f5` toward ADR-0070 directly; do
  not first recreate the ungated-`mod.rs` tree.
- **wasm-clippy is now load-bearing** for these components (UI type errors
  surface only on the wasm target — ADR-0070). A host-only `cargo check` will
  not catch them; use `cargo xtask check --no-test` (includes wasm-clippy) every
  task.
- **Default `cargo check` blind spot** still applies to the `#[server]` bodies
  under `feature="server"` — use `--all-features --all-targets` / the full gate.
- **First `component.rs` in the tree.** No template exists; ADR-0070 §Decision +
  style-guide §8 are the spec. Get the layout right here — later verticals copy
  it.

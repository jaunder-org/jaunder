# Spec — #316: converge the backup vertical onto the co-located Leptos layout

Issue: [#316](https://github.com/jaunder-org/jaunder/issues/316) — part of the
#303 umbrella. Governing decision:
`docs/adr/0056-web-canonical-colocated-leptos.md` (co-locate a feature's
`#[component]` UI, `#[server]` fns, and wire types in one module, split by cargo
`feature`, never `target_arch`).

## Context — what is already converged

The **server half of backup is already conformant** and needs no work:

- `web/src/backup/mod.rs` holds the four `#[server]` fns
  (`backup_warning_visible`, `current_user_is_operator`, `get_backup_settings`,
  `update_backup_settings`), imports the wire types from `common::backup`, and
  gates its server-only imports + the `require_operator` helper behind
  `#[cfg(feature = "server")]`. No `target_arch` gate.
- `web/src/backup/server.rs` holds the server-only `require_operator` helper +
  its test.
- Server-side bodies orchestrate `storage` traits inline under
  `feature = "server"` — the same pattern audiences/posts use. **This is
  idiomatic; it stays as-is.** Nothing moves to the `host` crate (that crate is
  for pure non-wasm cores — crypto, error carrier, metrics facade — not vertical
  storage-orchestration logic).

The **UI half is the outstanding work**. `BackupSettingsPage` +
`backup_settings_form` live in `web/src/pages/backup.rs`, inheriting the
implicit `#[cfg(target_arch = "wasm32")]` gate from the `pages` module
(`web/src/lib.rs:31`). The backup-domain banner `BackupBanner` lives in the
legacy `web/src/pages/ui.rs`.

## Goal

Co-locate backup's reactive UI with its server fns and wire types under
`web/src/backup/`, gated only by cargo `feature` (never `target_arch`), and
remove the `pages/backup.rs` counterpart — following the **audiences** vertical
as the template.

## Decisions (resolved in the design interview)

1. **UI lands in a co-located submodule `web/src/backup/ui.rs`** (not folded
   into `mod.rs`), keeping the reactive UI separable from the `#[server]` fns.
2. **`BackupBanner` is in scope** — it is backup-domain reactive UI consuming
   `backup_warning_visible()`, so it belongs in the backup vertical (advances
   the #312 `pages/ui.rs` dissolution). It moves into `web/src/backup/ui.rs`.
3. **The app-shell sidebar stays put.** `authed_sidebar` / the operator-gated
   sidebar chrome in `pages/ui.rs` is shell chrome, not backup-domain UI (#312
   relocates the shell in its own final increment). It remains in `pages/ui.rs`
   and keeps importing `current_user_is_operator` from `crate::backup`.
4. **`current_user_is_operator` is NOT relocated here.** Moving it to the `auth`
   vertical is the right home, but the `auth` vertical is under concurrent work
   (the `issue-315-auth-colocate` worktree). It stays in `web::backup` for this
   cycle; a follow-up issue captures the move.
5. **Server bodies stay inline, `feature = "server"`-gated** — no `host`
   extraction (see Context).

## Acceptance criteria

Each is observable so the ship conformance review can tell delivered from not.

- **AC1 — UI co-located.** `web/src/backup/ui.rs` exists and defines
  `BackupSettingsPage` and its `backup_settings_form` helper.
  `web/src/backup/mod.rs` declares the submodule and re-exports
  `BackupSettingsPage` so `crate::backup::BackupSettingsPage` resolves.
- **AC2 — `pages/backup.rs` gone.** The file `web/src/pages/backup.rs` no longer
  exists, and `web/src/pages/mod.rs` no longer declares `pub mod backup`.
- **AC3 — route repointed, unchanged behavior.** `web/src/pages/mod.rs` routes
  `/admin/backups` to `BackupSettingsPage` imported from `crate::backup` (not
  `crate::pages::backup`). The route path and the rendered form markup/behavior
  are preserved.
- **AC4 — no `target_arch` gate.** No `#[cfg(target_arch = ...)]` (or
  `cfg!(target_arch)`) is introduced on any relocated backup component. The UI
  module is unconditional; the `#[component]`s host-compile (dual-target,
  coverage-exempt) exactly as audiences' do.
- **AC5 — canonical Topbar.** The relocated `BackupSettingsPage` imports
  `Topbar` from `crate::ui`, not `crate::pages`.
- **AC6 — `BackupBanner` relocated.** `BackupBanner` is defined in
  `web/src/backup/ui.rs` (co-located with `backup_warning_visible`) and
  re-exported from `web/src/backup/mod.rs` so `crate::backup::BackupBanner`
  resolves; no `BackupBanner` definition remains in `web/src/pages/ui.rs`.
  `AppShell` renders `BackupBanner` imported from `crate::backup`. The
  now-dangling wiring in `pages/` is cleaned up: `BackupBanner` is dropped from
  the `pub use ui::{…}` re-export in `web/src/pages/mod.rs`, and the backup
  import at the top of `web/src/pages/ui.rs` narrows from
  `{backup_warning_visible, current_user_is_operator}` to just
  `current_user_is_operator` (the sidebar's remaining consumer).
- **AC7 — no fake-value host stub.** No fake-value host substitute is introduced
  for any wasm-only logic (ADR-0055 surviving principle). Relocated pure logic
  keeps its host-compiled home.
- **AC8 — server half unchanged.** The four `#[server]` fns and
  `web/src/backup/server.rs` keep their current inline,
  `feature = "server"`-gated shape; nothing moves to `host`.
- **AC9 — follow-up filed.** A GitHub issue exists proposing the relocation of
  `current_user_is_operator` from `web::backup` to the `auth` vertical, noting
  it is deferred behind the in-flight #315 work.
- **AC10 — gate green.** `cargo xtask validate` passes, including
  `end2end/tests/backup.spec.ts` (the three `/admin/backups` form-validation e2e
  tests) and a host-only `--all-features --all-targets` compile of the
  `server`-gated web code.

## Out of scope

- Relocating `current_user_is_operator` (AC9 follow-up).
- Relocating `authed_sidebar` / the app-shell chrome (the #312 shell increment).
- Any change to `common::backup`, the `server` crate's backup runner, or
  `host::metrics` backup emitters.

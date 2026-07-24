# Spec — #326 web(site): converge the site vertical + fold in #575 base-URL warn banner

**Issue:** [#326](https://github.com/jaunder-org/jaunder/issues/326) —
web(site): converge the site vertical onto the co-located Leptos layout.
**Folded in:** [#575](https://github.com/jaunder-org/jaunder/issues/575) — warn
banner when `site.base_url` is not configured. **Design floor:** ADR-0070
(four-file host/wasm split; supersedes ADR-0056), as re-scoped by #526 / #530.

## Context — current state

The `site` vertical is _half_-converged:

- `web/src/site/mod.rs` holds the two `#[server]` fns (`get_site_identity`,
  `update_site_identity`) inline — the pre-#530 shape (endpoints in `mod.rs`).
- `web/src/pages/site.rs` still holds the `SiteSettingsPage` component + its
  `site_settings_form` helper, wrapped in a `cov:ignore` block, importing the
  stale `crate::pages::Topbar`.

The canonical analog is the `backup` vertical (an operator-gated settings page):
`mod.rs` (wiring only) / `api.rs` (`#[server]` fns + wire types) /
`component.rs` (wasm-gated UI) / `server.rs` (host-only support). Five sibling
verticals (auth/media/posts/timeline/home) already shipped this layout.

Two facts steer the extra scope the maintainer directed:

1. `require_operator` — the operator-authorization guard both `backup` **and**
   `site` depend on — lives in `backup/server.rs` for historical reasons only.
   It is `require_auth().await?` + an `is_operator` check: an **auth guard**,
   sibling to `require_auth` (which lives in `auth/server.rs`). #575 would add a
   _third_ caller.
2. The #575 banner duplicates the `BackupBanner` structure (a `Resource` +
   `Suspense` + `role="alert"` div) and its `.j-backup-banner` styling. `#560`
   (require-base) is **closed**, so `base_url` is genuinely required for
   feeds/AtomPub now — the banner is timely.

## Scope

Four coherent workstreams, intended as separate commits (clean history):

### A. Site four-file convergence (mechanical)

Bring `site` to the ADR-0070 floor, mirroring `backup`:

- **`site/api.rs`** (new): the `#[server]` fns move here — `get_site_identity`,
  `update_site_identity`, and the new `base_url_warning_visible` (workstream B).
  Unconditional wire-type imports (`SiteTitle`, `AbsoluteUrl`, `SiteIdentity`);
  one grouped `#[cfg(feature = "server")]` use-block.
- **`site/component.rs`** (new): `SiteSettingsPage` + `site_settings_form` move
  here, plus the new `SiteBaseUrlBanner` wrapper (workstream D). Declared
  `#[cfg(target_arch = "wasm32")] mod component;` in `mod.rs`. **Zero
  `#[cfg(...)]` inside the file; zero `cov:ignore` markers** (wasm-gated ⇒
  coverage-exempt wholesale). Retarget `crate::pages::Topbar` →
  `crate::topbar::Topbar`.
- **`site/mod.rs`**: reduced to wiring only — a doc comment, the gated `mod`
  declarations, and re-exports (`pub use api::{…}`,
  `#[cfg(target_arch = "wasm32")] pub use component::{…}`). No items of its own.
  **No `site/server.rs`** (site owns no host-only support; `require_operator`
  moves to `auth` in workstream C).
- Delete `web/src/pages/site.rs`; update `web/src/pages/mod.rs` to drop
  `pub mod site;` and import `SiteSettingsPage` from `crate::site`.

No extractable host-tested signal logic exists — validation lives in the already
host-tested `Field<T>` / `ValidatedInput<T>` — so `component.rs` is entirely
wasm-gated with no ungated remainder.

### C. Relocate `require_operator` → `auth`

- Move `require_operator` (and its host test) from `backup/server.rs` to
  `auth/server.rs`, beside `require_auth`; re-export as
  `crate::auth::require_operator`.
- **Delete `backup/server.rs`** (it contained only this fn); drop
  `#[cfg(feature = "server")] pub(crate) mod server;` from `backup/mod.rs`.
- `backup/api.rs` and `site/api.rs` import `crate::auth::require_operator`. No
  remaining reference to `backup::server` anywhere in `web/src`.

### D. Generic `WarnBanner` component

- New wasm-only leaf module **`web::banner`** (`banner/mod.rs` +
  `banner/component.rs`, mirroring `topbar/`; component gated on the `mod`
  declaration). It owns the `Suspense` + `role="alert"` `.j-warn-banner`
  structure and renders only when its visibility resource resolves to `true`;
  message + action links are supplied by the caller.
- Rename the CSS class `.j-backup-banner` → **`.j-warn-banner`** in
  `server/assets/jaunder.css`; both banners use it. No `.j-backup-banner`
  reference remains.
- `BackupBanner` becomes a **thin wrapper**: it constructs its
  `backup_warning_visible()` resource and delegates structure/styling to
  `WarnBanner`. Its behavior and copy are unchanged.

### B. #575 — site base-URL warn banner

- **`base_url_warning_visible()`** server fn in `site/api.rs`: returns `true`
  iff the caller is an operator **and** `SiteIdentity.base_url` is `None`;
  returns `false` for non-operators. It mirrors `backup_warning_visible`'s
  **soft** operator check — an inline `require_auth` + `is_operator` that
  returns `Ok(false)` for a non-operator/unauthenticated caller. It must **not**
  call `require_operator` (that guard _errors_ on non-operators; the banner
  endpoint must degrade to "hidden", never surface an error).
- **`SiteBaseUrlBanner`** — a thin `WarnBanner` wrapper in `site/component.rs`.
  Copy: _"Site base URL is not configured — feeds and AtomPub are disabled."_
  with an action link to `/admin/site` (e.g. "Site Settings"). Hidden when
  `base_url` is set.
- Rendered in the authed `AppShell` (`pages/mod.rs`) alongside
  `<BackupBanner />`.

## Acceptance criteria

Layout / convergence (A):

- **AC1** `web/src/site/` contains exactly `mod.rs`, `api.rs`, `component.rs`
  (no `server.rs`). `web/src/pages/site.rs` does not exist.
- **AC2** `site/mod.rs` declares only gated `mod`s + re-exports (no
  `fn`/`struct`/ `#[server]`/`#[component]` items). The **only** `target_arch`
  gates in `web/src/site` are the two on the wasm component wiring — the
  `mod component;` declaration and its `pub use component::{…}` re-export —
  mirroring `backup/mod.rs`. No `target_arch` gate appears inside any
  `site/*.rs` body.
- **AC3** `site/component.rs` contains no `#[cfg(...)]` line and no `cov:ignore`
  marker, and imports `crate::topbar::Topbar`. No `crate::pages::Topbar`
  reference remains **within `web/src/site`** (the same import survives in the
  out-of-scope, parked `pages/sessions.rs` — #325 — and is not touched here).
- **AC4** `web/src/pages/mod.rs` no longer declares `pub mod site;`;
  `SiteSettingsPage` resolves via `crate::site`. Site settings page still loads
  and round-trips title/base_url (existing `admin-site.spec.ts` passes
  unchanged).
- **AC4b** No fake-value host stub is introduced anywhere in the changed code
  (ADR-0055): `component.rs` and `banner/component.rs` are wasm-only and never
  host-compiled, so there is no host `#[cfg(not(target_arch = "wasm32"))]` shim
  standing in for a component.

`require_operator` relocation (C):

- **AC5** `require_operator` is defined in `web/src/auth/server.rs` and callable
  as `crate::auth::require_operator`; its host test lives with it and passes.
  `web/src/backup/server.rs` does not exist; `rg 'backup::server' web/src`
  yields nothing. Backup + site both compile against
  `crate::auth::require_operator`.

Generic banner (D):

- **AC6** A `WarnBanner` component exists in `web::banner` (wasm-gated). Both
  `BackupBanner` and the site banner render through it.
  `rg 'j-backup-banner' web server/assets` yields nothing; the banner markup
  uses `.j-warn-banner`, which is defined once in `jaunder.css`. The backup
  banner's rendered markup — copy, both action links (`/admin/backups`,
  `/admin/site`), `role="alert"`, and the `.j-warn-banner` class — is
  byte-for-byte the same as before the extraction. (Structural equivalence is a
  **code-review** check: no e2e currently asserts the backup banner, and this
  cycle does not add one — see Out of scope.)

#575 banner behavior (B):

- **AC7** `base_url_warning_visible()` returns `true` for an operator when
  `site.base_url` is `None`, `false` when it is set, and `false` for a
  non-operator/unauthenticated caller. Verified by **dual-backend server
  integration tests** in `server/tests/web/web_site.rs` (POST
  `/api/base_url_warning_visible`, asserting the `"true"`/`"false"` body),
  mirroring `backup_warning_visible`'s tests in `server/tests/web/web_backup.rs`
  — the `#[server]` body "stays measured" (no coverage exemption;
  `xtask/src/coverage/exempt.rs::does_not_exempt_server_fn`), so this
  integration coverage is what the gate requires, not a host unit test.
- **AC8** In the authed admin surface, an operator sees the site base-URL banner
  (`role="alert" .j-warn-banner`, identified by its **copy text** — see the
  collision note) with a link to `/admin/site` when `base_url` is `None`, and
  does **not** see it when `base_url` is set. The new e2e (in
  `end2end/tests/admin-site.spec.ts`) **drives both states explicitly within the
  test** rather than relying on a seed default: set `base_url` to a known URL
  via the settings UI → assert the site banner is **absent**; then clear it via
  the UI (the ADR-0065 clear-via-omit path, which dispatches `base_url: None`) →
  assert the site banner is **present**. This routes the "unset" state through
  the proven UI clear-path, sidestepping any question of whether a
  CLI/`seedConfigViaTool` empty-string maps to `None`.
  - **Shared-class collision (must-honor in assertions):** after the
    `.j-warn-banner` rename, the backup banner is _also_
    `role="alert" .j-warn-banner`, and it is visible for operators whenever the
    backup destination is unset (the e2e seed default). Therefore every banner
    assertion here (present **and** absent) must locate the site banner by its
    **copy text** ("Site base URL is not configured…"), never by
    `.j-warn-banner`/`role=alert` alone — those would match the backup banner.
- **AC9** A non-operator does not see the site base-URL banner. Verified at the
  **integration level** — `base_url_warning_visible` returns `false` for a
  non-operator/unauthenticated caller (AC7's tests), and the banner renders
  purely as a function of that resource (`Ok(true)` only). No non-operator e2e
  is added: non-operators are denied `/admin/site` outright (existing
  `admin-site.spec.ts`), so an in-browser absence check would add flake surface
  for no coverage the integration test lacks.

Global:

- **AC10** `cargo xtask validate` is green, including the site e2e flows across
  all `{sqlite,postgres}×{chromium,firefox}` combos.

## Out of scope

- **#609** (flaky `admin-site.spec.ts:47` base_url round-trip on sqlite/firefox)
  — a pre-existing parallel-load timing flake; not this cycle (maintainer chose
  to leave it). If the new e2e proves flaky under the same conditions, note it,
  don't chase it.
- **#330** `pages/` dissolution beyond removing `pages/site.rs` — App/Router
  stay put; this cycle only deletes the one page file and rewires its import.
- **#420** leptosfmt mangling `<ValidatedInput<T>>` — a formatter bug; leave the
  awkward wrapping as leptosfmt produces it.
- Adding a backup-banner e2e — the new site-banner e2e exercises the shared
  `WarnBanner` path; backup's coverage is unchanged.

## Decisions / ADRs

No new ADR. The four-file layout is ADR-0070; the `require_operator` relocation
and `WarnBanner` extraction are locality/DRY refactors within that established
structure, not novel architectural decisions.

## Verification

- Host gate: `devtool run -- cargo xtask check` while iterating;
  `--all-features --all-targets` reachable (server-gated site code).
- Wasm clippy on `component.rs` / `banner/component.rs` before committing
  wasm-only code.
- Full local gate: `cargo xtask validate` (incl. e2e). New banner e2e must pass
  on all four combos.

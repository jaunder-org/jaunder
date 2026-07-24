# #326 web(site) convergence + #575 base-URL banner — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating an individual task to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-24-issue-326-site-colocate.md`](../specs/2026-07-24-issue-326-site-colocate.md)
— read it; this plan is "how," the spec is "what/why." Task/AC references point
back to it.

**Goal:** Bring the `site` vertical to the ADR-0070 four-file layout, relocate
the shared `require_operator` guard to `auth`, extract a generic `WarnBanner`,
and add the #575 base-URL warning banner.

**Architecture:** Four independent commits, ordered so each compiles and gates
green on its own: (1) move `require_operator` to `auth`; (2) split `site` into
`mod/api/component`; (3) hoist `WarnBanner` into a new `web::banner` leaf +
refactor `BackupBanner`; (4) add `base_url_warning_visible` +
`SiteBaseUrlBanner`. `#[server]` fns are exercised by dual-backend integration
tests (`server/tests/web/`); the wasm-only UI is covered by the browser e2e; no
host stubs.

**Tech Stack:** Rust, Leptos 0.7 (CSR), `#[server]` fns, `rstest`/`rstest_reuse`
dual-backend integration tests, Playwright e2e, `cargo xtask` gate.

## Global Constraints

_(copied from the spec — every task's requirements implicitly include these)_

- **ADR-0070 four-file floor:** `mod.rs` = wiring only (gated `mod` decls +
  re-exports, no items); `api.rs` = `#[server]` fns + wire types; `component.rs`
  = wasm-only UI, declared `#[cfg(target_arch = "wasm32")] mod component;`,
  **zero `#[cfg(...)]` inside, zero `cov:ignore`**; `server.rs` only if the
  vertical owns host-only support (site does **not**).
- **No fake host stub** (ADR-0055) — components are wasm-only, never
  host-compiled.
- **`target_arch` gates** appear only on the component `mod` declaration + its
  `pub use` re-export (mirror `backup/mod.rs:4,13`).
- **`#[server]` bodies stay coverage-measured**
  (`exempt.rs::does_not_exempt_server_fn`) — cover new endpoints with
  dual-backend integration tests, not `cov:ignore`.
- **Banner copy (site):** _"Site base URL is not configured — feeds and AtomPub
  are disabled."_ Backup copy unchanged: _"Backups are not configured. Your data
  is at risk."_
- **Banner CSS class:** single shared `.j-warn-banner` (rename from
  `.j-backup-banner`); no `.j-backup-banner` reference remains.
- **No `Co-Authored-By` trailer.** One clean commit per task; run
  `cargo xtask check` before each commit (`jaunder-commit`).
- **Wasm-only code** (`component.rs`, `banner/component.rs`): run
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` before
  committing — `cargo check`/`build` skip wasm clippy.

**Separable concerns:** none to file. The out-of-scope items (#609 flake, #330
`pages/` dissolution, #420 leptosfmt) already have tracking issues; nothing new
is split out of this cycle.

**Commands (exact):**

- Web host unit tests (server-gated):
  `cargo nextest run -p web --features server <filter>`
- Integration tests (dual-backend):
  `cargo nextest run -p jaunder --test integration <filter>`
- Wasm clippy:
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
- Iterating gate: `devtool run -- cargo xtask check`
- Site e2e (local): `cargo xtask e2e-local admin-site`
- Full local gate: `cargo xtask validate`

## Review header — task summary

| Task  | Deliverable                                                                                                            | Key AC               |
| ----- | ---------------------------------------------------------------------------------------------------------------------- | -------------------- |
| **1** | Relocate `require_operator` → `auth/server.rs`; delete `backup/server.rs`; kill site→backup coupling                   | AC5                  |
| **2** | Split `site` into `mod.rs`/`api.rs`/`component.rs`; delete `pages/site.rs`; retarget Topbar; drop `cov:ignore`         | AC1–4, AC4b          |
| **3** | New `web::banner` leaf with generic `WarnBanner`; `BackupBanner` → thin wrapper; `.j-backup-banner` → `.j-warn-banner` | AC6                  |
| **4** | `base_url_warning_visible()` + `SiteBaseUrlBanner` rendered in `AppShell`; integration + e2e coverage                  | AC7–9, AC8-collision |

**Key risks / decisions:**

- **Shared-class collision (Task 4 e2e):** after the rename both banners are
  `role="alert" .j-warn-banner`, and the backup banner is _also_ visible for
  operators (backup destination unset by default). Every banner e2e assertion
  locates the site banner by **copy text**, never by class/role alone.
- **`WarnBanner` link prop:** links are a `Vec<(&'static str, &'static str)>`
  data prop — there is **no `ChildrenFn` precedent** anywhere in `web/src`, so a
  children slot is not introduced.
- **Endpoint stability (Task 2):** `#[server(endpoint = "…")]` fixes the URL;
  re-exports from `mod.rs` keep `crate::site::…` call sites and the registrar
  paths stable, so the existing `web_site.rs` integration tests +
  `admin-site.spec.ts` pass unchanged after the move.
- **`base_url` default is `None`** (`web_site.rs:61`), so the "warning visible"
  state needs no seeding in integration tests.

---

## Task 1: Relocate `require_operator` from `backup` to `auth`

**Files:**

- Modify: `web/src/auth/server.rs` (add `require_operator` + its test),
  `web/src/auth/mod.rs:52-53` (re-export)
- Delete: `web/src/backup/server.rs`
- Modify: `web/src/backup/mod.rs` (drop `mod server;`),
  `web/src/backup/api.rs:9` (import path), `web/src/site/mod.rs:6-7` (import
  path — the pre-convergence file; Task 2 restructures it)
- Test: `web/src/auth/server.rs` `#[cfg(test)] mod tests` (existing module at
  ~line 194)

**Interfaces:**

- Consumes: `require_auth` (same file, `auth/server.rs`);
  `crate::error::{InternalError, InternalResult}`; `storage::UserStorage`;
  `crate::test_support::auth_parts` (test only).
- Produces: `crate::auth::require_operator` —
  `pub async fn require_operator() -> InternalResult<()>` (errors `Unauthorized`
  for non-operator/absent user). Backup + site both import it from here.

- [x] **Step 1: Move the test to `auth/server.rs`**, adapted from
      `backup/server.rs:21-50`. Append into the existing
      `#[cfg(test)] mod tests` (add the imports it needs — they may partly
      overlap what's already there):

```rust
    // Add ONLY these to the existing `mod tests` use-block. `provide_context` and
    // `Owner` are ALREADY imported there (`use leptos::prelude::{provide_context, Owner};`,
    // ~line 198) — do NOT re-import them (E0252/E0255).
    use crate::test_support::auth_parts;
    use common::ids::UserId;
    use std::sync::Arc;
    use storage::{MockUserStorage, UserStorage};

    // guard:no-backend — mock store
    #[tokio::test]
    async fn require_operator_rejects_when_user_absent() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "ghost"));
        let mut users = MockUserStorage::new();
        users.expect_get_user().returning(|_uid| Ok(None));
        provide_context(Arc::new(users) as Arc<dyn UserStorage>);

        let result = require_operator().await;
        drop(owner);
        let err = result.unwrap_err();
        assert!(matches!(
            crate::error::project(err.kind(), err.public_message()),
            crate::error::WebError::Unauthorized
        ));
    }
```

- [x] **Step 2: Run it, verify it FAILS** (no `require_operator` in `auth` yet):

Run:
`cargo nextest run -p web --features server require_operator_rejects_when_user_absent`
Expected: FAIL — `require_operator` not found in this scope.

- [x] **Step 3: Move the function** from `backup/server.rs:7-19` into
      `auth/server.rs` (verbatim body — it is `require_auth().await?` + a
      `UserStorage` `is_operator` check). Add two file-scope imports:
      `use storage::UserStorage;` and `use leptos::prelude::expect_context;` —
      note `auth/server.rs` does **not** import `leptos::prelude::*` (it uses
      fully-qualified `leptos::context::use_context` elsewhere), so
      `expect_context` is genuinely absent and must be added. `Arc`,
      `InternalError`, `InternalResult` are already imported. **Delete
      `web/src/backup/server.rs`.**

```rust
pub async fn require_operator() -> InternalResult<()> {
    let auth = require_auth().await?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    let Some(user) = users.get_user(auth.user_id).await? else {
        return Err(InternalError::unauthorized("user does not exist"));
    };
    if !user.is_operator {
        return Err(InternalError::unauthorized("operator access required"));
    }
    Ok(())
}
```

- [x] **Step 4: Rewire imports/exports.**
  - `auth/mod.rs:53`: add `require_operator` to
    `pub use server::{require_auth, AuthRejection, AuthUser, CookieSettings};` →
    `{require_auth, require_operator, AuthRejection, AuthUser, CookieSettings}`.
  - `backup/mod.rs`: delete the line
    `#[cfg(feature = "server")] pub(crate) mod server;`.
  - `backup/api.rs:9`: `use super::server::require_operator;` →
    `use crate::auth::require_operator;`.
  - `site/mod.rs` (pre-convergence file): the single
    `use crate::backup::server::require_operator;` import line (`site/mod.rs:7`,
    shared by both server fns) → `use crate::auth::require_operator;`.
  - Verify none remain: `rg 'backup::server' web/src` → empty.

- [x] **Step 5: Run the test + gate, verify PASS**

Run:
`cargo nextest run -p web --features server require_operator_rejects_when_user_absent`
Expected: PASS. Then: `devtool run -- cargo xtask check` → green (also compiles
backup + site against the new path).

- [x] **Step 6: Commit**

```bash
git add web/src/auth/server.rs web/src/auth/mod.rs web/src/backup/mod.rs web/src/backup/api.rs web/src/site/mod.rs
git rm web/src/backup/server.rs
git commit -m "refactor(web): move require_operator from backup to auth (#326)"
```

---

## Task 2: Converge `site` onto the four-file layout

**Files:**

- Create: `web/src/site/api.rs`, `web/src/site/component.rs`
- Rewrite: `web/src/site/mod.rs` (wiring only)
- Delete: `web/src/pages/site.rs`
- Modify: `web/src/pages/mod.rs` (drop `pub mod site;`; import
  `SiteSettingsPage` from `crate::site`)
- Regression: `server/tests/web/web_site.rs` (unchanged — must still pass);
  `end2end/tests/admin-site.spec.ts` (unchanged — must still pass)

**Interfaces:**

- Consumes: `crate::auth::require_operator` (Task 1); `crate::topbar::Topbar`;
  `crate::forms::{Field, ValidatedInput}`;
  `crate::error::{WebError, WebResult, InternalError}`;
  `common::site::{SiteIdentity, SiteTitle}`;
  `common::absolute_url::AbsoluteUrl`; `storage::SiteConfigStorage`.
- Produces:
  `crate::site::{get_site_identity, update_site_identity, GetSiteIdentity, UpdateSiteIdentity}`
  (re-exported from `api`); `crate::site::SiteSettingsPage` (wasm-only,
  re-exported from `component`). Paths unchanged for `pages/mod.rs` and the
  server-fn registrar.

_This task is a structural move with no new behavior; its "tests" are the
**existing** `web_site.rs` integration suite and `admin-site.spec.ts` e2e, which
must pass unchanged (endpoints are stable). No new host test._

- [x] **Step 1: Create `site/api.rs`** — move both `#[server]` fns out of the
      current `site/mod.rs` verbatim (they already carry
      `#[server(endpoint = "…")]`, `#[tracing::instrument]`, `boundary!`).
      Structure the imports like `backup/api.rs`: unconditional wire types + one
      grouped `#[cfg(feature = "server")]` block.

```rust
use crate::error::WebResult;
use common::absolute_url::AbsoluteUrl;
use common::site::{SiteIdentity, SiteTitle};
use leptos::prelude::*;

#[cfg(feature = "server")]
use {
    crate::auth::require_operator,
    crate::error::InternalError,
    std::sync::Arc,
    storage::SiteConfigStorage,
};

// … get_site_identity and update_site_identity, moved verbatim …
```

- [x] **Step 2: Create `site/component.rs`** — move `SiteSettingsPage` +
      `site_settings_form` from `pages/site.rs` verbatim, with three edits: (a)
      `use crate::pages::Topbar;` → `use crate::topbar::Topbar;`; (b)
      `use crate::site::{get_site_identity, UpdateSiteIdentity};` →
      `use super::{get_site_identity, UpdateSiteIdentity};`; (c) **delete the
      `// cov:ignore-start` / `// cov:ignore-stop` lines** around
      `site_settings_form`. No `#[cfg(...)]` appears in the file.

- [x] **Step 3: Rewrite `site/mod.rs`** to wiring only (mirror `backup/mod.rs`):

```rust
//! Site settings vertical: operator-gated site-identity endpoints + the
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{get_site_identity, update_site_identity, GetSiteIdentity, UpdateSiteIdentity};
#[cfg(target_arch = "wasm32")]
pub use component::SiteSettingsPage;
```

- [x] **Step 4: Delete `pages/site.rs`; fix `pages/mod.rs`.** Remove
      `pub mod site;` (line 3). Change
      `use crate::pages::site::SiteSettingsPage;` (line 25) →
      `use crate::site::SiteSettingsPage;`. Leave `pub mod sessions;` / the
      sessions import untouched (out of scope, #325).

- [x] **Step 5: Gate + regression, verify PASS**

Run: `cargo nextest run -p jaunder --test integration web_site` → PASS
(unchanged endpoints). Run:
`cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` → clean.
Run: `devtool run -- cargo xtask check` → green. Confirm the layout ACs by
inspection: `rg -c 'target_arch' web/src/site` → 2 (mod decl + re-export);
`rg 'cov:ignore|crate::pages::Topbar' web/src/site` → empty. Run:
`cargo xtask e2e-local admin-site` → PASS (site settings page still
loads/round-trips).

- [x] **Step 6: Commit**

```bash
git add web/src/site/ web/src/pages/mod.rs
git rm web/src/pages/site.rs
git commit -m "refactor(web/site): converge onto the ADR-0070 four-file layout (#326)"
```

---

## Task 3: Extract a generic `WarnBanner`; refactor `BackupBanner`; rename the CSS class

**Files:**

- Create: `web/src/banner/mod.rs`, `web/src/banner/component.rs`
- Modify: `web/src/lib.rs` (declare `mod banner;` — gated, near the other leaf
  modules)
- Modify: `web/src/backup/component.rs:186-211` (`BackupBanner` → thin
  `WarnBanner` wrapper)
- Modify: `server/assets/jaunder.css` (rename `.j-backup-banner` →
  `.j-warn-banner`)

**Interfaces:**

- Consumes: `crate::error::WebResult`; `leptos::prelude::*`.
- Produces: `crate::banner::WarnBanner` (wasm-only) —
  `#[component] pub fn WarnBanner(visible: Resource<WebResult<bool>>, message: &'static str, links: Vec<(&'static str, &'static str)>) -> impl IntoView`.
  Renders a sticky `role="alert" .j-warn-banner` only when `visible` resolves to
  `Ok(true)`; `links` are `(href, label)` pairs.

_`WarnBanner` and `BackupBanner` are wasm-only components (not host-compiled),
so there is no host unit test; correctness is covered by wasm-clippy + the
backup rendering staying structurally identical (code review, AC6) + the Task 4
e2e that drives the shared `WarnBanner` path. `.j-warn-banner` is the same rule
as today, only renamed._

- [x] **Step 1: Create `web/src/banner/component.rs`** with the generic
      component. Body mirrors `backup/component.rs:190-209` but parameterized:

```rust
use crate::error::WebResult;
use leptos::prelude::*;

/// A sticky operational warning bar (`role="alert"`, `.j-warn-banner`) shown only
/// when `visible` resolves to `Ok(true)`. `message` is the copy; `links` are
/// `(href, label)` action links. Wasm-only (server-fn-driven `Resource`).
#[component]
pub fn WarnBanner(
    visible: Resource<WebResult<bool>>,
    message: &'static str,
    links: Vec<(&'static str, &'static str)>,
) -> impl IntoView {
    view! {
        <Suspense fallback=|| ()>
            {move || {
                let links = links.clone();
                Suspend::new(async move {
                    match visible.await {
                        Ok(true) => {
                            let items = links
                                .iter()
                                .map(|(href, label)| view! { <a href=*href>{*label}</a> })
                                .collect_view();
                            view! {
                                <div class="j-warn-banner" role="alert">
                                    <span>{message}</span>
                                    <div>{items}</div>
                                </div>
                            }
                            .into_any()
                        }
                        _ => ().into_any(),
                    }
                })
            }}
        </Suspense>
    }
}
```

- [x] **Step 2: Create `web/src/banner/mod.rs`** (wiring only, wasm-only leaf —
      mirror `topbar/mod.rs` shape without the pure-render twin):

```rust
//! Shared operational warning banner (`WarnBanner`) — the sticky `role="alert"`
//! `.j-warn-banner` bar used by the backup and site verticals. Wasm-only.
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::WarnBanner;
```

- [x] **Step 3: Declare the module** in `web/src/lib.rs` — add an **ungated**
      `mod banner;` alongside the other leaf `mod`s (e.g. near `topbar`/`icon`,
      which are declared ungated with the wasm-gating internal to their own
      `mod.rs`). `banner/mod.rs` gates its `component`/`WarnBanner` internally,
      so host-side `crate::banner` is an empty module — which is correct, since
      nothing host-side references it. This mirrors the leaf convention
      faithfully.

- [x] **Step 4: Refactor `BackupBanner`** (`backup/component.rs:186-211`) to
      delegate to `WarnBanner` — same resource, same copy, same two links, same
      rendered markup:

```rust
#[component]
pub fn BackupBanner() -> impl IntoView {
    let visible = Resource::new(|| (), |()| backup_warning_visible());
    view! {
        <crate::banner::WarnBanner
            visible=visible
            message="Backups are not configured. Your data is at risk."
            links=vec![("/admin/backups", "Configure Backups"), ("/admin/site", "Site Settings")]
        />
    }
}
```

- [x] **Step 5: Rename the CSS class.** In `server/assets/jaunder.css`, rename
      the `.j-backup-banner` selector (and any descendant selectors keyed off
      it, e.g. `.j-backup-banner a`) to `.j-warn-banner`. Verify:
      `rg 'j-backup-banner' web server/assets` → empty.

- [x] **Step 6: Wasm-clippy + gate, verify PASS**

Run: `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` →
clean. Run: `devtool run -- cargo xtask check` → green. Review: the
`BackupBanner` rendered markup (copy, both links, `role="alert"`,
`.j-warn-banner`) is unchanged from before (AC6).

- [x] **Step 7: Commit**

```bash
git add web/src/banner/ web/src/lib.rs web/src/backup/component.rs server/assets/jaunder.css
git commit -m "refactor(web): extract generic WarnBanner; rename .j-backup-banner to .j-warn-banner (#326)"
```

---

## Task 4: `base_url_warning_visible()` + `SiteBaseUrlBanner` (#575)

**Files:**

- Modify: `web/src/site/api.rs` (add `base_url_warning_visible`)
- Modify: `web/src/site/component.rs` (add `SiteBaseUrlBanner`),
  `web/src/site/mod.rs` (re-exports)
- Modify: `web/src/pages/mod.rs` (`AppShell`: render `<SiteBaseUrlBanner />`;
  import it)
- Test: `server/tests/web/web_site.rs` (integration);
  `end2end/tests/admin-site.spec.ts` (e2e)

**Interfaces:**

- Consumes: `crate::auth::require_auth` + `storage::UserStorage` (soft operator
  check); `storage::SiteConfigStorage` (`get_identity`);
  `crate::banner::WarnBanner` (Task 3).
- Produces: `crate::site::{base_url_warning_visible, BaseUrlWarningVisible}`;
  `crate::site::SiteBaseUrlBanner` (wasm-only). Endpoint
  `/api/base_url_warning_visible`.

- [x] **Step 1: Write the failing integration tests** in
      `server/tests/web/web_site.rs` (dual-backend, mirroring `web_backup.rs`'s
      `backup_warning_*`). Add to the existing `use` block:
      `create_user_and_session` is already imported; no new key constant is
      needed (default `base_url` is `None`).

```rust
#[apply(backends)]
#[tokio::test]
async fn base_url_warning_visible_for_operator_when_unset(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator").await.cookie();
    let (status, body) =
        post_form(state, "/api/base_url_warning_visible", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "true");
}

#[apply(backends)]
#[tokio::test]
async fn base_url_warning_hidden_when_base_url_configured(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator").await.cookie();
    let (up, up_body) = post_form(
        Arc::clone(&state),
        "/api/update_site_identity",
        "title=My+Blog&base_url=https%3A%2F%2Fexample.com%2F",
        Some(&cookie),
    )
    .await;
    assert_eq!(up, StatusCode::OK, "body: {up_body}");
    let (status, body) =
        post_form(state, "/api/base_url_warning_visible", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

#[apply(backends)]
#[tokio::test]
async fn base_url_warning_hidden_for_non_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state, "member").await.cookie();
    let (status, body) =
        post_form(state, "/api/base_url_warning_visible", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

#[apply(backends)]
#[tokio::test]
async fn base_url_warning_hidden_without_authentication(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let (status, body) =
        post_form(state, "/api/base_url_warning_visible", "", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body, "false");
}

// Covers the Err(non-Auth) branch of the endpoint body — required because #[server]
// bodies stay coverage-measured (AC7). Mirrors web_backup.rs's
// backup_warning_visible_propagates_storage_error_during_auth: close the pool after
// session creation so authenticate() returns Internal (not Auth) → 500.
#[apply(backends)]
#[tokio::test]
async fn base_url_warning_propagates_storage_error_during_auth(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator").await.cookie();
    base.close_pool().await;
    let (status, _body) = post_form(
        Arc::clone(&state),
        "/api/base_url_warning_visible",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
```

- [x] **Step 2: Run them, verify they FAIL**

Run: `cargo nextest run -p jaunder --test integration base_url_warning`
Expected: FAIL — endpoint `/api/base_url_warning_visible` not registered
(404/non-OK).

- [x] **Step 3: Implement `base_url_warning_visible`** in `site/api.rs`. Soft
      operator check (mirror `backup_warning_visible`: `Ok(false)` for
      non-operator/unauthenticated — **not** `require_operator`), then
      `base_url.is_none()`. Add `require_auth`, `ErrorKind`, `UserStorage` to
      the file's `#[cfg(feature = "server")]` use-block.

```rust
#[server(endpoint = "/base_url_warning_visible")]
#[tracing::instrument(name = "web.site.base_url_warning_visible")]
pub async fn base_url_warning_visible() -> WebResult<bool> {
    boundary!("base_url_warning_visible", {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if error.kind() == ErrorKind::Auth => return Ok(false),
            Err(error) => return Err(error),
        };
        let users = expect_context::<Arc<dyn UserStorage>>();
        let is_operator = users
            .get_user(auth.user_id)
            .await?
            .is_some_and(|u| u.is_operator);
        if !is_operator {
            return Ok(false);
        }
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        Ok(site_config.get_identity().await?.base_url.is_none())
    })
}
```

Re-export it from `site/mod.rs`: add
`base_url_warning_visible, BaseUrlWarningVisible` to the `pub use api::{…}`
list.

- [x] **Step 4: Run the integration tests, verify PASS**

Run: `cargo nextest run -p jaunder --test integration base_url_warning`
Expected: PASS (all four, both backends).

- [x] **Step 5: Add `SiteBaseUrlBanner`** to `site/component.rs` and render it
      in `AppShell`.

In `site/component.rs`:

```rust
#[component]
pub fn SiteBaseUrlBanner() -> impl IntoView {
    let visible = Resource::new(|| (), |()| super::base_url_warning_visible());
    view! {
        <crate::banner::WarnBanner
            visible=visible
            message="Site base URL is not configured — feeds and AtomPub are disabled."
            links=vec![("/admin/site", "Site Settings")]
        />
    }
}
```

Re-export from `site/mod.rs`:
`#[cfg(target_arch = "wasm32")] pub use component::{SiteBaseUrlBanner, SiteSettingsPage};`.
In `pages/mod.rs`: add `SiteBaseUrlBanner` to the `use crate::site::…` import,
and render `<SiteBaseUrlBanner />` immediately after `<BackupBanner />` (in
`AppShell`, ~line 58).

- [x] **Step 6: Write the e2e** in `end2end/tests/admin-site.spec.ts` (drive
      both states explicitly; disambiguate by **copy text**, per AC8's collision
      note). Reuse the file's `login`/`waitForSelector`/`fill` helpers.

```ts
test("site base URL warning banner shows when unset and hides once configured", async ({
  page,
}) => {
  await login(page, "testoperator", "testpassword123");
  const banner = page.getByText("Site base URL is not configured");

  // Set base_url via the settings UI → banner hidden. Use the file's `goto` helper
  // (imported alongside `login`/`waitForSelector`), consistent with the other tests.
  await goto(page, "/admin/site");
  await waitForSelector(page, "input[name='base_url']");
  await page.fill('input[name="base_url"]', "https://example.com");
  await page.getByRole("button", { name: "Save Site Settings" }).click();
  await expect(page.getByText("Site settings saved.")).toBeVisible();
  await page.reload();
  await expect(banner).toBeHidden();

  // Clear base_url via the UI omit-path → banner visible.
  await page.fill('input[name="base_url"]', "");
  await page.getByRole("button", { name: "Save Site Settings" }).click();
  await expect(page.getByText("Site settings saved.")).toBeVisible();
  await page.reload();
  await expect(banner).toBeVisible();
});
```

**AC9 (non-operator sees no banner) is covered at the integration level**, not
e2e: `base_url_warning_hidden_for_non_operator` (Step 1) pins the endpoint to
`"false"`, and the banner renders as a pure function of that resource
(`Ok(true)` only). A non-operator e2e is not added — non-operators are denied
`/admin/site` outright (existing `admin-site.spec.ts` test), so an in-browser
non-operator banner-absence check would add flake surface (cf. #609) for no
coverage the integration test doesn't already give.

- [x] **Step 7: Verify + gate.**

Run: `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` →
clean. Run: `devtool run -- cargo xtask check` → green. Run:
`cargo xtask e2e-local admin-site` → PASS (new banner test + existing site
tests).

- [x] **Step 8: Commit**

```bash
git add web/src/site/ web/src/pages/mod.rs server/tests/web/web_site.rs end2end/tests/admin-site.spec.ts
git commit -m "feat(web/site): warn banner when site base_url is unconfigured (#575, #326)"
```

---

## Final verification (before ship)

- [x] `cargo xtask validate` → green across all
      `{sqlite,postgres}×{chromium,firefox}` e2e combos (AC10).
- [x] Spec ACs re-checked: `rg 'backup::server' web/src` empty (AC5);
      `rg -c target_arch web/src/site` = 2 and `rg 'cov:ignore' web/src/site`
      empty (AC2/AC3); `rg 'j-backup-banner' web server/assets` empty (AC6);
      `web/src/backup/server.rs` and `web/src/pages/site.rs` gone (AC1/AC5).
- [x] `git diff wt-base-issue-326..HEAD --stat` reviewed — four commits, no
      stray files.

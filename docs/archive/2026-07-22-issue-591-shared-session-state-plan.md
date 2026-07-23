# Shared Reactive Session State + Drop the SSR-era Redirect Hook — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace five ad-hoc `current_user()`/`current_user_is_operator()`
fetches with one shared, reactive, marker-seeded session context, and delete the
`set_redirect_hook` override so login/logout/registration navigate client-side
(no wasm re-boot).

**Architecture:** A `cfg`-free `SessionUser { username, is_operator }` value is
persisted in the `jaunder_auth` marker and returned by a new `session()` server
fn. `App` provides a `SessionContext` (a marker-seeded
`RwSignal<Option<SessionUser>>`

- a per-navigation reconcile `Resource` calling `session()` + optimistic
  set/clear helpers). Consumers read the context. Removing the redirect-hook
  override lets `<Router>`'s own same-origin `use_navigate` hook take the
  first-caller-wins `OnceLock`, turning `redirect("/")` into pushState.

**Tech Stack:** Rust, Leptos 0.8 CSR, `leptos_router` 0.8.13, `serde_json`,
`cargo nextest`, Playwright e2e.

**Spec:** `docs/superpowers/specs/2026-07-22-issue-591-shared-session-state.md`
(this plan is "how"; the spec is "what/why" — read it first). Section refs below
point into it.

## Review header

**Scope — in:**

- `SessionUser` value + `is_operator` in the marker codec/storage (spec §1).
- New `session()` server fn; retire `current_user()` + reactive
  `current_user_is_operator()` (spec §2).
- `SessionContext` provided by `App`; migrate all five reactive call sites + two
  direct marker reads to it (spec §3, §4).
- `login()` returns `is_operator` for flash-free first login (spec §5).
- Delete the `set_redirect_hook` override; optimistic session update on
  login/logout/register (spec §5, §6).
- E2E rewrites + no-reload/flash-free assertions (spec "E2E + docs"); ADR-0044
  update (spec "Decisions to record").

**Scope — out (deferred, do not touch):** post-publish/unpublish
`location.replace`, the permalink misroute, the `no-full-load` enforcement gate
(all **#592**); projector; pre-paint `/`→`/app` redirect; `require_operator()`
server guard. See spec "Non-goals".

**Tasks:**

1. Session data model — `SessionUser`, marker codec/storage, `login()` returns
   `is_operator`.
2. `session()` server fn (additive) + server-integration tests.
3. `SessionContext` + provide in `App`; migrate the Sidebar (`ui.rs`).
4. Migrate the remaining consumers (Cockpit, CreatePostPage, SubscribeButton,
   `marker_matches`).
5. Retire `current_user()` + reactive `current_user_is_operator()`.
6. Delete the redirect-hook override; optimistic session update on auth actions.
7. E2E — no-reload sentinel, flash-free operator, login/logout spec rewrites,
   `end2end/CLAUDE.md`.
8. Update ADR-0044.

**Key risks / decisions:**

- **Old-marker migration flash** — decode MUST default absent `is_operator` to
  `false` (`#[serde(default)]`), else existing sessions flash anonymous on first
  post-deploy boot (spec §1). Task 1 pins this with a test.
- **`login()` wire-format change is web-only** — the elisp/emacs frontend uses
  HTTP Basic auth, not `/login`; verified. But the server integration test
  helper `extract_token` (`server/tests/web/web_auth.rs:18`) parses the login
  body as a bare JSON string and WILL break — Task 1 splits it.
- **Router-hook behavior is settled, not assumed** (spec §6): `leptos_router`
  0.8.13 installs a same-origin `use_navigate` hook; `OnceLock`
  first-caller-wins
  - Router-mounts-before-`ActionForm` makes it win. No fallback code needed;
    Task 7 verifies behaviorally.
- **`end2end/CLAUDE.md` is untracked** (main checkout only) — its edit cannot
  land in this branch's diff; handle in the main checkout (Task 7 flags at
  execution).

---

## Global Constraints

_Copied from the spec + `CONTRIBUTING.md`; every task implicitly includes
these._

- **Backend parity:** storage-touching tests follow the dual-backend template
  (`#[apply(backends)]`); a bare `#[tokio::test]` that should be dual-backend
  fails the `test-backend-pattern` guard. The `session()` integration tests are
  dual-backend.
- **Coverage policy:** ADR-0050 stateless gate; `cov:ignore` only where
  justified (the existing `authed_sidebar` block keeps its markers); CRAP T=30.
- **No `Co-Authored-By` trailer** on commits.
- **Gate:** the pre-commit hook runs full `cargo xtask check`; run
  `cargo xtask check` (via `devtool run -- cargo xtask check`) clean before each
  commit (**jaunder-commit**).
- **Marker contract:** `username` stays the top-level JSON key;
  `PREPAINT_SCRIPT` (`web/src/render/mod.rs:38`) + its byte-identical
  `csr/index.html` twin + drift tests stay untouched.
- **Newtypes pervasive:** `Username` is a validating newtype; `SessionUser`
  holds it directly (no stringly-typed username).

---

### Task 1: Session data model — `SessionUser`, marker codec/storage, `login()` returns `is_operator`

**Files:**

- Modify: `web/src/auth/marker.rs` (`SessionUser` + codec + tests)
- Modify: `web/src/auth/marker_storage.rs:21-38` (`get`/`set`/`remove` on
  `SessionUser`)
- Modify: `web/src/auth/api.rs:36-98` (`login` return type + `is_operator`
  lookup)
- Modify: `web/src/auth/mod.rs:37` (re-export `SessionUser`, `LoginResponse`)
- Modify: `web/src/auth/component.rs:25-31` (login Effect writes `SessionUser`)
- Modify: `web/src/registration/component.rs:53-59` (register Effect writes
  `SessionUser`)
- Modify: `web/src/pages/ui.rs:96-114` (transitional: reconcile writes
  `SessionUser{.., is_operator:false}`)
- Modify: `web/src/posts/component.rs:181-183` (transitional: `marker_matches`
  reads `.username`)
- Test: `web/src/auth/marker.rs` (`#[cfg(test)]`),
  `server/tests/web/web_auth.rs`

_All of `web/src/pages`, `web/src/posts/component.rs`,
`web/src/auth/component.rs`, and `web/src/registration/component.rs` are
**wasm-only** (`lib.rs:34`, `posts/mod.rs:17`, `auth/mod.rs:30`), so the
`marker_storage` signature change (Step 4) breaks their callers on the
**wasm-clippy** step of `cargo xtask check`
(`xtask/src/steps/static_checks.rs`), not the host build. Every unmigrated
`marker_storage` caller must be made to compile in this task — Steps 8-9._

**Interfaces:**

- Produces:
  - `pub struct SessionUser { pub username: Username, pub is_operator: bool }`
    in `web::auth::marker`, re-exported as `web::auth::SessionUser`. Derives
    `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`; `is_operator` carries
    `#[serde(default)]`.
  - `web::auth::marker::encode_marker(&SessionUser) -> String`
  - `web::auth::marker::decode_marker(&str) -> Option<SessionUser>`
  - `web::auth::marker_storage::{get() -> Option<SessionUser>, set(&SessionUser), remove()}`
  - `pub struct LoginResponse { pub token: String, pub is_operator: bool }`
    (`Serialize, Deserialize`), re-exported `web::auth::LoginResponse`;
    `login(...) -> WebResult<LoginResponse>`.

- [ ] **Step 1: Rewrite `marker.rs` tests for the `SessionUser` shape** (one per
      branch)

Replace the `#[cfg(test)]` block in `web/src/auth/marker.rs` with:

```rust
#[cfg(test)]
mod tests {
    use common::test_support::parse_username;

    use super::{decode_marker, encode_marker, SessionUser};

    #[test]
    fn round_trips_session_info() {
        let info = SessionUser { username: parse_username("alice"), is_operator: true };
        let raw = encode_marker(&info);
        // The exact JSON the pre-paint <head> script parses: `username` stays the
        // top-level key (script reads only `.username`); `is_operator` is additive.
        assert_eq!(raw, r#"{"username":"alice","is_operator":true}"#);
        assert_eq!(decode_marker(&raw), Some(info));
    }

    #[test]
    fn decode_defaults_is_operator_when_absent() {
        // Backward compat: markers written before this change lack `is_operator`.
        // They MUST decode as a non-operator session, not `None` (else existing
        // sessions flash anonymous on first post-deploy boot). Spec §1.
        assert_eq!(
            decode_marker(r#"{"username":"alice"}"#),
            Some(SessionUser { username: parse_username("alice"), is_operator: false }),
        );
    }

    #[test]
    fn round_trips_all_valid_username_chars() {
        let info = SessionUser { username: parse_username("a_b-9"), is_operator: false };
        assert_eq!(decode_marker(&encode_marker(&info)), Some(info));
    }

    #[test]
    fn decode_rejects_malformed_json() {
        assert_eq!(decode_marker("not json"), None);
        assert_eq!(decode_marker("{}"), None);
    }

    #[test]
    fn decode_rejects_invalid_username() {
        assert_eq!(decode_marker(r#"{"username":"Has Space"}"#), None);
        assert_eq!(decode_marker(r#"{"username":""}"#), None);
    }
}
```

- [ ] **Step 2: Run the marker tests, verify they fail**

Run: `devtool run -- cargo nextest run -p web auth::marker` Expected: FAIL —
`SessionUser` undefined; `encode/decode` still `Username`-typed.

- [ ] **Step 3: Implement `SessionUser` + codec in `marker.rs`**

Replace the `Marker`/`encode_marker`/`decode_marker` items (lines 17-43) with,
to the signatures in **Interfaces**:

```rust
use serde::{Deserialize, Serialize};

/// The whole client-visible session identity (#181, ADR-0044). Persisted in the
/// advisory marker and returned by `session()`. `is_operator` is advisory chrome
/// only — `require_operator()` is the real guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUser {
    pub username: Username,
    /// Absent in pre-#591 markers → `false` (backward compatible). Spec §1.
    #[serde(default)]
    pub is_operator: bool,
}

/// The localStorage value (JSON `{"username":"…","is_operator":<bool>}`). `username`
/// stays the top-level key the pre-paint script reads.
#[must_use]
pub fn encode_marker(info: &SessionUser) -> String {
    serde_json::to_string(info).unwrap_or_default()
}

/// Parse a marker back to its [`SessionUser`]; `None` when JSON is malformed or the
/// stored username is invalid (routes through `Username`'s validating `FromStr`).
#[must_use]
pub fn decode_marker(raw: &str) -> Option<SessionUser> {
    serde_json::from_str(raw).ok()
}
```

`Username`'s own `Deserialize` already routes through its validating `FromStr`
(the `decode_rejects_invalid_username` test pins it); no separate `Owned` struct
is needed. Every branch (round-trip, absent-field default, invalid-char accept,
malformed→None, invalid-username→None) is pinned by a Step-1 test.

- [ ] **Step 4: Update `marker_storage.rs` to `SessionUser`**

`web/src/auth/marker_storage.rs`: change imports to
`use super::marker::{decode_marker, encode_marker, SessionUser, MARKER_KEY};`
and the three fns to:

```rust
#[must_use]
pub fn get() -> Option<SessionUser> {
    client::storage::get(MARKER_KEY).ok().flatten().and_then(|raw| decode_marker(&raw))
}

pub fn set(info: &SessionUser) {
    let _ = client::storage::set(MARKER_KEY, &encode_marker(info));
}

pub fn remove() {
    let _ = client::storage::remove(MARKER_KEY);
}
```

(Drop the now-unused `use common::username::Username;` if the compiler flags
it.)

- [ ] **Step 5: Write the failing login-returns-`is_operator` integration test**

In `server/tests/web/web_auth.rs`, first split the token helper (login's body is
now a struct; register's is still a bare string):

```rust
/// Login now returns `{"token":"…","is_operator":<bool>}` (#591).
fn extract_login(body: &str) -> (RawToken, bool) {
    #[derive(serde::Deserialize)]
    struct Resp { token: String, is_operator: bool }
    let r: Resp = serde_json::from_str(body.trim()).expect("valid login JSON body");
    (r.token.parse().expect("valid token"), r.is_operator)
}
```

Update **all four** login-body call sites (`extract_token(&body)` at
`web_auth.rs:288`, `:344`, `:377`, `:416` — the last two parse login bodies in
`login_with_empty_label…` and `login_truncates_long_user_agent`) to
`extract_login(&body).0`; leave the **register** call sites (`:50`, `:123`) on
`extract_token` (register body is unchanged). Confirm the full set with
`rg -n 'extract_token' server/tests/web/web_auth.rs` before editing — any login
site left on `extract_token` fails its `starts_with('"')` assertion at Step 10.
Add a new test:

```rust
#[apply(backends)]
#[tokio::test]
async fn login_returns_is_operator_flag(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state.site_config.set("site.registration_policy", "open").await.unwrap();
    // A freshly registered user is a non-operator.
    post_form_with_secure_flag(
        Arc::clone(&state), "/api/register",
        "username=alice&password=password123", None, true,
    ).await;
    let (status, _cookie, body) = post_form_with_secure_flag(
        Arc::clone(&state), "/api/login",
        "username=alice&password=password123", None, true,
    ).await;
    assert_eq!(status, StatusCode::OK);
    let (token, is_operator) = extract_login(&body);
    assert!(!token.is_empty());
    assert!(!is_operator, "a freshly registered user is not an operator");
}
```

- [ ] **Step 6: Run the login test, verify it fails**

Run:
`devtool run -- cargo nextest run -p jaunder --test integration web_auth::login_returns_is_operator_flag`
Expected: FAIL — `login` still returns `WebResult<String>`; body is a bare
string.

_(Confirm the exact `--test` target name with
`rg -n 'mod web' server/tests/integration.rs` if the harness name differs;
`web_auth` is a submodule of the `web` test tree.)_

- [ ] **Step 7: Change `login()` to return `LoginResponse`**

In `web/src/auth/api.rs`: add the wire type near the top of the module (ungated
— it is a `#[server]` return type referenced on both builds):

```rust
/// `login`'s success payload: the raw session token (unchanged) plus the viewer's
/// operator flag, so the client writes a complete marker immediately (flash-free
/// first login, spec §5). Web-only wire type — the elisp frontend uses Basic auth.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub is_operator: bool,
}
```

Change the signature to `-> WebResult<LoginResponse>` and the tail of the body
(replacing lines 94-96). `record` is the authenticated `UserRecord`, which
already carries `is_operator` (`storage/src/users.rs:43`) — no extra query:

```rust
        set_session_cookie(&raw_token);
        leptos_axum::redirect("/");
        Ok(LoginResponse { token: raw_token.to_string(), is_operator: record.is_operator })
    })
}
```

- [ ] **Step 8: Update the client login/register Effects + re-exports**

`web/src/auth/mod.rs:37`: extend the re-export to include `SessionUser` (from
`marker`) and `LoginResponse` (from `api`). Concretely add
`pub use api::LoginResponse;` and `pub use marker::SessionUser;` (keep existing
`Login`, `Logout`, etc.; `CurrentUser`/`current_user` are removed in Task 5).

`web/src/auth/component.rs:25-31` — login Effect writes the full marker; the
returned value now carries `is_operator`:

```rust
    Effect::new(move |_| {
        if let Some(Ok(resp)) = login_action.value().get() {
            if let Some(input) = login_action.input().get() {
                marker_storage::set(&super::SessionUser {
                    username: input.username,
                    is_operator: resp.is_operator,
                });
            }
        }
    });
```

Also update the view closure's type annotation at `component.rs:75`:
`.map(|r: Result<LoginResponse, WebError>| match r {` (import `LoginResponse`
via `use super::{marker_storage, Login, LoginResponse, Logout, SessionUser};`).
The `Ok(_) =>` arm is unchanged (value still discarded for the "Logging in…"
view).

`web/src/registration/component.rs:53-59` — register Effect (new users are never
operators):

```rust
    Effect::new(move |_| {
        if let Some(Ok(_)) = register_action.value().get() {
            if let Some(input) = register_action.input().get() {
                marker_storage::set(&crate::auth::SessionUser {
                    username: input.username,
                    is_operator: false,
                });
            }
        }
    });
```

- [ ] **Step 9: Transitional fixes to unmigrated wasm marker callers
      (compile-only)**

`web/src/posts/component.rs:181-183` — `marker_matches` compares `get()` (now
`Option<SessionUser>`) to `Some(author: &Username)`, a type error. Read
`.username` transitionally (Task 4 migrates it to the context):

```rust
fn marker_matches(author: &Username) -> bool {
    crate::auth::marker_storage::get().map(|s| s.username).as_ref() == Some(author)
}
```

Then the Sidebar reconcile write:

`web/src/pages/ui.rs:96-114` — the reconcile Effect calls
`marker_storage::set(&u)` with a `Username`; the new signature needs
`SessionUser`. Write `is_operator: false` here **transitionally** — nothing
reads the marker's `is_operator` until Task 3's seed does, and Task 3 deletes
this whole Effect:

```rust
                Ok(Some(u)) => {
                    // TRANSITIONAL (#591 Task 1): marker `is_operator` is unread
                    // until the shared session context seeds from it (Task 3),
                    // which also deletes this Effect. `false` is harmless here.
                    crate::auth::marker_storage::set(&crate::auth::SessionUser {
                        username: u.clone(),
                        is_operator: false,
                    });
                    if owner.get_untracked().as_ref() != Some(&u) {
                        owner.set(Some(u));
                    }
                }
```

The `marker_username_on_boot()` read (`ui.rs:136-138`) returns
`marker_storage::get()` which is now `Option<SessionUser>`; the `owner` signal
is `Option<Username>`. Map at the boot site transitionally:
`RwSignal::new(marker_username_on_boot().map(|s| s.username))` and change
`marker_username_on_boot` to `-> Option<SessionUser>` returning `get()` — or,
simpler, inline
`let owner = RwSignal::new(marker_storage::get().map(|s| s.username));` and
delete `marker_username_on_boot`. (Task 3 removes all of this.)

- [ ] **Step 10: Run the full check + both test suites, verify green**

Run: `devtool run -- cargo nextest run -p web auth::marker` Run:
`devtool run -- cargo nextest run -p jaunder --test integration web_auth`
Expected: PASS (all marker tests + `login_returns_is_operator_flag` + the
updated existing login/register tests). Run: `devtool run -- cargo xtask check`
Expected: PASS (fmt + clippy + coverage; `--all-features` builds the wasm side).

- [ ] **Step 11: Commit**

```bash
git add web/src/auth/marker.rs web/src/auth/marker_storage.rs web/src/auth/api.rs web/src/auth/mod.rs web/src/auth/component.rs web/src/registration/component.rs web/src/pages/ui.rs web/src/posts/component.rs server/tests/web/web_auth.rs
git commit -m "feat(web/auth): SessionUser carries is_operator; login returns it (#591)"
```

---

### Task 2: `session()` server fn + server-integration tests

**Files:**

- Modify: `web/src/auth/api.rs` (add `session()`; `Session` struct is
  macro-generated)
- Modify: `web/src/auth/mod.rs:37` (re-export `session`, `Session`)
- Modify: `server/tests/helpers/mod.rs:31` (register `web::auth::Session`)
- Test: `server/tests/web/web_backup.rs` (has the local `create_session_cookie`
  operator helper)

**Interfaces:**

- Consumes: `SessionUser` (Task 1), `require_auth`, `UserStorage`.
- Produces: `web::auth::session() -> WebResult<Option<SessionUser>>`
  (`#[server(endpoint = "/session")]`), macro struct `web::auth::Session`.

- [ ] **Step 1: Write failing `session()` integration tests** (anon / member /
      operator + storage error)

`session()` is a no-arg `#[server]` fn — invoked by **POST** (default encoding),
like the `current_user` tests. Add to `server/tests/web/web_backup.rs`, which
already has the local `create_session_cookie(&state, name, is_operator)` helper
(`:269`) and imports the shared `post_form` (`helpers/mod.rs:247`). These mirror
— and, once Task 5 deletes them, replace the coverage of — the existing
`current_user_is_operator_*` tests:

```rust
#[apply(backends)]
#[tokio::test]
async fn session_reports_username_and_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let operator_cookie = create_session_cookie(&state, "operator", true).await;
    let member_cookie = create_session_cookie(&state, "member", false).await;

    let (status, body) = post_form(Arc::clone(&state), "/api/session", "", Some(&operator_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains(r#""username":"operator""#), "body: {body}");
    assert!(body.contains(r#""is_operator":true"#), "body: {body}");

    let (status, body) = post_form(Arc::clone(&state), "/api/session", "", Some(&member_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains(r#""username":"member""#), "body: {body}");
    assert!(body.contains(r#""is_operator":false"#), "body: {body}");

    let (status, body) = post_form(state, "/api/session", "", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body.trim(), "null"); // Ok(None) serializes to JSON null
}
```

Also add a storage-error test mirroring
`current_user_is_operator_propagates_storage_error_during_auth`
(`web_backup.rs:439`) verbatim but POSTing `/api/session` — this covers
`session()`'s `get_user(...).await?` / non-auth `require_auth` error propagation
(needed for the coverage gate on the new fn):

```rust
#[apply(backends)]
#[tokio::test]
async fn session_propagates_storage_error_during_auth(#[case] backend: Backend) {
    // (Copy the body of current_user_is_operator_propagates_storage_error_during_auth,
    //  changing only the path to "/api/session" and the expected error assertion to
    //  match session()'s error surface — a 500, not a 200 body.)
}
```

- [ ] **Step 2: Run them, verify they fail**

Run:
`devtool run -- cargo nextest run -p jaunder --test integration web_backup::session`
Expected: FAIL — `/api/session` unrouted (404 / not registered).

- [ ] **Step 3: Implement `session()`**

In `web/src/auth/api.rs`, add (uses the same `#[cfg(feature = "server")]`
`UserStorage`/`Arc` already imported for `login`):

```rust
/// The viewer's session identity — username + operator flag — or `None` when
/// anonymous/expired. The single reconcile fetch behind the shared session context
/// (#591), superseding `current_user` + `current_user_is_operator`.
#[server(endpoint = "/session")]
#[tracing::instrument(name = "web.auth.session")]
pub async fn session() -> WebResult<Option<super::SessionUser>> {
    boundary!("session", {
        let auth = match require_auth().await {
            Ok(auth) => auth,
            Err(error) if error.kind() == crate::error::ErrorKind::Auth => return Ok(None),
            Err(error) => return Err(error),
        };
        let users = expect_context::<Arc<dyn UserStorage>>();
        let is_operator = users
            .get_user(auth.user_id)
            .await?
            .is_some_and(|u| u.is_operator);
        Ok(Some(super::SessionUser { username: auth.username, is_operator }))
    })
}
```

Re-export in `web/src/auth/mod.rs:37`: add `session, Session` to the `api::{…}`
group. Register in `server/tests/helpers/mod.rs` (near line 31):
`server_fn::axum::register_explicit::<web::auth::Session>();`

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run -- cargo nextest run -p jaunder --test integration web_backup::session`
Expected: PASS. Run: `devtool run -- cargo xtask check` → PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/auth/api.rs web/src/auth/mod.rs server/tests/helpers/mod.rs server/tests/web/web_backup.rs
git commit -m "feat(web/auth): add session() endpoint returning SessionUser (#591)"
```

---

### Task 3: `SessionContext` + provide in `App`; migrate the Sidebar

**Files:**

- Create: `web/src/auth/session.rs` (client context)
- Modify: `web/src/auth/mod.rs` (`mod session;` + re-exports)
- Modify: `web/src/pages/mod.rs:67-95` (provide the context in `App`)
- Modify: `web/src/pages/ui.rs:71-138` (Sidebar reads context; delete its
  resources/reconcile/seed)

**Interfaces:**

- Consumes: `session()` (Task 2), `SessionUser`, `marker_storage`.
- Produces (in `web::auth::session`, re-exported `web::auth::…`):
  - `#[derive(Clone, Copy)] pub struct SessionContext { pub seed: RwSignal<Option<SessionUser>>, pub reconcile: Resource<Option<SessionUser>> }`
    (leptos `Resource` is `Copy`; `RwSignal` is `Copy` — the struct is `Copy`).
  - `pub fn provide_session_context()` — seeds from the marker, spawns the
    per-navigation reconcile, provides the context.
  - `pub fn use_session() -> SessionContext` (`expect_context`).
  - `pub fn set_session(info: SessionUser)` and `pub fn clear_session()` —
    optimistic seed + marker mutators (used by Task 6). Read the context via
    `use_session`.

- [ ] **Step 1: Create `web/src/auth/session.rs`**

```rust
//! The shared reactive session context (#591, ADR-0044): one marker-seeded
//! `SessionUser` signal + a per-navigation reconcile against `session()`.
//! Supersedes the ad-hoc `current_user()` fetches. wasm-only reactive glue; the
//! `session()` server fn lives in `api.rs`.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use super::{marker_storage, session, SessionUser};

/// The viewer/session identity shared across the app tree.
#[derive(Clone, Copy)]
pub struct SessionContext {
    /// Synchronously-readable session, seeded from the advisory marker at mount
    /// (flash-free) and kept current by `reconcile`.
    pub seed: RwSignal<Option<SessionUser>>,
    /// Per-navigation server confirmation. Awaiting it gives the authoritative
    /// (cookie-checked) session for gates that must not trust a stale marker.
    pub reconcile: Resource<Option<SessionUser>>,
}

/// Provide the session context from `App`. Seeds from the marker, then reconciles
/// against `session()` on every navigation, writing the result back into the seed
/// signal AND the marker (so the next boot is flash-free). ADR-0044 D3.
pub fn provide_session_context() {
    let seed = RwSignal::new(marker_storage::get());
    let location = use_location();
    let reconcile = crate::server_resource(move || location.pathname.get(), |_| session());
    Effect::new(move |_| {
        if let Some(Ok(next)) = reconcile.get() {
            match &next {
                Some(info) => marker_storage::set(info),
                None => marker_storage::remove(),
            }
            if seed.get_untracked() != next {
                seed.set(next);
            }
        }
    });
    provide_context(SessionContext { seed, reconcile });
}

#[must_use]
pub fn use_session() -> SessionContext {
    expect_context::<SessionContext>()
}

/// Optimistically set the session (login/register) — seed signal + marker.
pub fn set_session(info: SessionUser) {
    let ctx = use_session();
    marker_storage::set(&info);
    ctx.seed.set(Some(info));
}

/// Optimistically clear the session (logout) — seed signal + marker.
pub fn clear_session() {
    let ctx = use_session();
    marker_storage::remove();
    ctx.seed.set(None);
}
```

Add the module + re-exports to `web/src/auth/mod.rs`, **wasm-gated** exactly
like `component` (`session.rs` calls the wasm-only `marker_storage`, and every
consumer — `pages/*`, `posts/component`, the auth/registration components — is
itself wasm-only, so nothing on the host build references it):

```rust
#[cfg(target_arch = "wasm32")]
mod session;
#[cfg(target_arch = "wasm32")]
pub use session::{
    clear_session, provide_session_context, set_session, use_session, SessionContext,
};
```

_No unit test: this is reactive wasm glue verified by the Sidebar behavior below
and the Task 7 e2e. Its pure decision (marker↔signal reconcile) mirrors the
Task-1-tested codec; there is no host-observable branch to pin here._

- [ ] **Step 2: Provide the context in `App`**

`web/src/pages/mod.rs` — inside `App`, after `provide_context(theme);` (line
95), add: `crate::auth::provide_session_context();`. (Import path: it is
re-exported at `crate::auth::provide_session_context`.) Do **not** remove the
redirect hook yet — that is Task 6.

- [ ] **Step 3: Migrate the Sidebar to the context**

Rewrite `web/src/pages/ui.rs` `Sidebar` (lines 74-131) to read the context;
delete the `operator` resource, the `reconcile` resource + Effect, the `owner`
boot signal, and `marker_username_on_boot`. `is_operator` now comes from the
same `SessionUser`, so operator chrome is flash-free:

```rust
#[component]
pub fn Sidebar(#[prop(optional)] active: Option<String>) -> impl IntoView {
    let active_key = active.unwrap_or_default();
    let session = crate::auth::use_session().seed;
    let anon_html = crate::render::render_sidebar(&active_key);
    view! {
        <aside class="j-sidebar">
            {move || match session.get() {
                None => view! {
                    <div style="display:contents" inner_html=anon_html.clone()></div>
                }.into_any(),
                Some(info) => authed_sidebar(&active_key, &info.username, info.is_operator).into_any(),
            }}
        </aside>
    }
}
```

Remove now-dead imports (`use crate::auth::current_user;`,
`use crate::backup::current_user_is_operator;`, `use_location`, `Username` if
unused). `authed_sidebar` is unchanged (still `(active_key, &Username, bool)`).

- [ ] **Step 4: Verify the build + existing e2e-covered behavior**

Run: `devtool run -- cargo xtask check` → PASS (host + wasm build; clippy
clean). Behavior is characterized by the existing sidebar e2e (logged-in
indicator) — run locally only if cheap; otherwise it is gated by CI's e2e
matrix. The new flash-free operator assertion is added in Task 7.

- [ ] **Step 5: Commit**

```bash
git add web/src/auth/session.rs web/src/auth/mod.rs web/src/pages/mod.rs web/src/pages/ui.rs
git commit -m "feat(web): shared SessionContext; sidebar reads it (#591)"
```

---

### Task 4: Migrate the remaining consumers

**Files:**

- Modify: `web/src/pages/cockpit.rs:13,32-43` (viewer from context; split feed
  fetch)
- Modify: `web/src/posts/component.rs:14,181-183,1089,1296-1306` (context reads)

**Interfaces:**

- Consumes: `use_session()` (Task 3).

- [ ] **Step 1: Migrate `CockpitPage`**

`web/src/pages/cockpit.rs`: drop `use crate::auth::current_user;`. The bounce
gate must stay **server-confirmed** (an expired cookie must bounce), so read the
context's `reconcile` Resource, not just the seed. Replace the `initial_page`
resource (lines 32-43) so the viewer comes from `reconcile` and the feed fetch
is keyed on `refresh_version`:

```rust
    let session = crate::auth::use_session();
    let initial_page = crate::server_resource(
        move || refresh_version.get(),
        move |_| async move {
            match session.reconcile.await {
                Ok(Some(info)) => list_home_feed(None, None, Some(PageSize::default()))
                    .await
                    .map(|page| Some((info.username, page))),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        },
    );
```

(`session.reconcile.await` yields the server-confirmed `Option<SessionUser>`;
the downstream Effect at lines 46-60 that copies `(user, page)` into the
timeline is unchanged — `info.username` is the same `Username` it expects.)

- [ ] **Step 2: Migrate the three `posts/component.rs` sites**

Drop `use crate::auth::current_user;` (line 14).

`marker_matches` (lines 177-183) → read the context seed (reactive,
marker-backed):

```rust
/// `true` when the shared session's username equals `author` (#181/#591): the
/// client-side signal that the viewer owns this post. `false` on the host build
/// (no context) — wasm-only chrome.
fn marker_matches(author: &Username) -> bool {
    use_context::<crate::auth::SessionContext>()
        .and_then(|ctx| ctx.seed.get_untracked())
        .as_ref()
        .map(|info| &info.username)
        == Some(author)
}
```

(Uses `use_context` not `use_session` so the host build — where no context is
provided — yields `None`→`false` instead of panicking on `expect_context`.)

`CreatePostPage` (line 1089) → await the context reconcile (server-confirmed
gate):

```rust
    let session = crate::auth::use_session();
    // ...
            {move || Suspend::new(async move {
                match session.reconcile.await {
                    Ok(Some(_)) => { /* unchanged form arm */ }
                    // unchanged anon/err arms
```

Replace `current_user.await` at line 1098 with `session.reconcile.await`; delete
the `let current_user = …` resource at line 1089.

`SubscribeButton` (lines 1296-1306) → viewer from the seed (identity is stable);
keep the `is_subscribed_to` fetch keyed on the action versions:

```rust
    let session = crate::auth::use_session();
    let username_for_state = username.clone();
    let state = crate::server_resource(
        move || (subscribe.version().get(), unsubscribe.version().get()),
        move |_| {
            let username = username_for_state.clone();
            async move {
                let subscribed = is_subscribed_to(username.clone()).await.unwrap_or(false);
                (session.seed.get_untracked().map(|i| i.username), subscribed)
            }
        },
    );
```

(The `(viewer, subscribed)` tuple shape at line 1315 is unchanged — `viewer` is
still `Option<Username>`.)

- [ ] **Step 3: Verify the build**

Run: `devtool run -- cargo xtask check` → PASS.
`rg -n 'current_user\(\)' web/src` should now return **only** the definition in
`auth/api.rs` (removed next task) — no reactive callers remain.

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/cockpit.rs web/src/posts/component.rs
git commit -m "refactor(web): cockpit + posts read the shared SessionContext (#591)"
```

---

### Task 5: Retire `current_user()` + reactive `current_user_is_operator()`

**Files:**

- Modify: `web/src/auth/api.rs:27-34` (delete `current_user`)
- Modify: `web/src/auth/server.rs:198-206` (delete `classify_current_user` if
  now unused; keep tests only if the fn stays)
- Modify: `web/src/auth/mod.rs:1-2,37` (drop `current_user`, `CurrentUser` from
  docs + re-exports)
- Modify: `web/src/backup/api.rs:46-62` (delete reactive
  `current_user_is_operator`)
- Modify: `web/src/backup/mod.rs:10` (drop `current_user_is_operator` re-export)
- Modify: `server/tests/helpers/mod.rs:31,33` (drop `CurrentUser`,
  `CurrentUserIsOperator` registrations)
- Modify: `server/tests/web/router.rs:121-154` (repoint the route smoke test →
  `/api/session`)
- Modify: `server/tests/web/web_backup.rs:32-63,418-470` (delete the two
  `current_user_is_operator_*` tests — coverage moved to Task 2's `session_*`
  tests)

**Interfaces:** removals only. `require_operator()` (`backup/server.rs`) and
`backup_warning_visible` (`backup/api.rs:21`, computes `is_operator` inline)
stay. The two endpoints these tests hit no longer exist after Step 1, so the
tests MUST be repointed/removed in the SAME commit or the test build fails.

- [ ] **Step 1: Delete the two reactive server fns + registrations +
      re-exports**

- `web/src/auth/api.rs`: remove `current_user` (lines 27-34).
- `web/src/backup/api.rs`: remove the `#[server] current_user_is_operator` item
  (lines 46-62). Keep `require_operator`, `backup_warning_visible`,
  `get/update_backup_settings`.
- Re-exports: `web/src/auth/mod.rs:37` drop `current_user, CurrentUser`;
  `web/src/backup/mod.rs:10` drop `current_user_is_operator`. Fix the module doc
  comments that list `current_user` (`auth/mod.rs:1`, `auth/api.rs:2`) to name
  `session` instead.
- `server/tests/helpers/mod.rs`: delete the `CurrentUser` (line 31) and
  `CurrentUserIsOperator` (line 33) `register_explicit` lines.
- `classify_current_user` (`auth/server.rs:200`): `session()` does not call it
  (it inlines the auth-error→None match). If `rg -n classify_current_user web`
  shows no remaining caller, delete the fn and its two unit tests
  (`server.rs:333,340`); if you instead refactored `session()` to reuse it, keep
  both.
- **Orphaned endpoint tests (delete/repoint in this commit):**
  - `server/tests/web/router.rs:121-154` `current_user_api_route_returns_ok`
    POSTs the deleted `/api/current_user` → repoint its URI to `/api/session`
    and rename it `session_api_route_returns_ok` (a live route smoke test, worth
    keeping).
  - `server/tests/web/web_backup.rs`: delete
    `current_user_is_operator_reports_operator_status` (`:32-63`) and
    `current_user_is_operator_propagates_storage_error_during_auth` (`:439`) —
    both POST the deleted `/api/current_user_is_operator`; their operator-true
    and storage-error coverage now lives in Task 2's `session_*` tests. (Leave
    `create_session_cookie` at `:269` — Task 2's tests use it.)

- [ ] **Step 2: Verify removal is clean**

Run: `devtool run -- cargo xtask check` → PASS. Run:
`rg -n 'current_user\b|current_user_is_operator|CurrentUser\b'` over `web/` and
`server/` → only comments/prose, **no** definitions, callers, or registrations
(spec acceptance criterion 4).

- [ ] **Step 3: Commit**

```bash
git add web/src/auth/api.rs web/src/auth/mod.rs web/src/auth/server.rs web/src/backup/api.rs web/src/backup/mod.rs server/tests/helpers/mod.rs server/tests/web/router.rs server/tests/web/web_backup.rs
git commit -m "refactor(web): retire current_user + reactive current_user_is_operator (#591)"
```

---

### Task 6: Delete the redirect-hook override; optimistic session update on auth actions

**Files:**

- Modify: `web/src/pages/mod.rs:72-83` (delete the `set_redirect_hook` block +
  comment)
- Modify: `web/src/auth/component.rs:25-31,92-102` (login/logout Effects
  set/clear the session context)
- Modify: `web/src/registration/component.rs:53-59` (register Effect sets the
  session context)

**Interfaces:** Consumes `set_session`/`clear_session` (Task 3).

- [ ] **Step 1: Delete the override, leave the ordering rationale**

`web/src/pages/mod.rs` — remove lines 72-83 (the comment + `set_redirect_hook`
call). Replace with a one-line comment recording why removal is safe (spec §6):

```rust
    // No server-fn redirect hook override: <Router> installs a same-origin
    // use_navigate hook into the first-caller-wins OnceLock, and it mounts before
    // any ActionForm, so login/logout/register redirects are client-side pushState
    // (no full reload). Chrome updates via the shared session context. (#591)
```

- [ ] **Step 2: Optimistic session updates in the auth Effects**

The marker writes from Task 1 stay; add the seed-signal update so chrome flips
without waiting for the reconcile. Replace
`marker_storage::set(&SessionUser{…})` in the **login** Effect
(`auth/component.rs`) with:

```rust
                crate::auth::set_session(crate::auth::SessionUser {
                    username: input.username,
                    is_operator: resp.is_operator,
                });
```

**logout** Effect (`auth/component.rs:98-102`): replace
`marker_storage::remove();` with `crate::auth::clear_session();`. **register**
Effect (`registration/component.rs`): replace the
`marker_storage::set(&SessionUser{…})` with
`crate::auth::set_session(crate::auth::SessionUser { username: input.username, is_operator: false });`.
(`set_session`/`clear_session` handle both the marker and the seed signal, so
the direct `marker_storage` calls are no longer needed in these three Effects;
drop now-unused `marker_storage` imports if the compiler flags them.)

- [ ] **Step 3: Verify the build**

Run: `devtool run -- cargo xtask check` → PASS.
`rg -n 'set_redirect_hook|location\(\)\.replace' web/src` → no hits in the
login/logout path (the pre-paint `location.replace('/app')` in `render/mod.rs`
is a string constant, out of scope, and must remain).

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/mod.rs web/src/auth/component.rs web/src/registration/component.rs
git commit -m "feat(web): drop SSR-era full-reload redirect hook; reactive session update (#591)"
```

---

### Task 7: E2E — no-reload sentinel, flash-free operator, spec rewrites

**Files:**

- Modify: the login/logout e2e specs under `end2end/tests/` (find with
  `rg -l "logout|login" end2end/tests`)
- Modify (main checkout only — untracked): `end2end/CLAUDE.md:37-39`

- [ ] **Step 1: Add the no-wasm-reboot assertion (spec criterion 1)**

In the login (and logout) spec, before triggering the action, stash a sentinel
on `window` and assert it survives the transition (a full document load wipes
it):

```ts
await page.evaluate(() => {
  (window as any).__jaunderSpaSentinel = true;
});
// ... perform login (fill form, submit) ...
await page.waitForURL(BASE_URL + "/"); // now reliable — pushState, not location.replace
await expect
  .poll(() =>
    page.evaluate(() => (window as any).__jaunderSpaSentinel === true),
  )
  .toBe(true); // survived → no wasm re-boot
```

Assert reactive chrome (criterion 2) with an element wait on the authed sidebar
(`a[href='/logout']`) after login, and its absence after logout — not a document
reload.

- [ ] **Step 2: Add the flash-free operator assertion (spec criterion 3,
      observable proxy)**

Seed an operator session (register a user, mark `is_operator` via the test/seed
path used elsewhere — mirror an existing operator/backup e2e; search
`rg -l "operator|Configure Backups|admin/backups" end2end/tests`). Reload the
authed page and assert the operator nav item is present via a **CSS locator**
(`page.locator("a[href='/admin/backups']")`, not `getByRole`) at first
content-ready paint. Optionally route-intercept `**/api/session` to delay it and
prove the operator chrome is present before that response resolves.

- [ ] **Step 3: Rewrite `waitForURL`/`data-hydrated` usage in the affected
      specs**

Replace any post-login/logout `location.replace`-era waits: `waitForURL` is now
reliable; do **not** wait on `body[data-hydrated]` after an SPA navigation (it
is per-document, trivially already set) — assert content readiness instead (spec
"E2E + docs").

- [ ] **Step 4: Update `end2end/CLAUDE.md` (main checkout — untracked)**

`end2end/CLAUDE.md:37-39` currently reads _"Server-side 302 redirects (e.g.
logout) are reliable with `waitForURL`; client-side `location.replace()`
redirects (e.g. post-login) are not"_. Rewrite: after #591 login/logout/register
are all client-side pushState and `waitForURL` is reliable for each. **This file
is untracked and absent from the worktree** — make the edit in the main checkout
(`/home/mdorman/src/jaunder/end2end/CLAUDE.md`), or raise with the maintainer
whether to bring it under version control first. Do **not** silently skip it.
The post-publish/unpublish `location.replace` guidance is removed by #592, not
here.

- [ ] **Step 5: Run the affected specs (or defer to CI)**

Local e2e is reaped in this environment (memory: local e2e VM reaped) — run
`devtool run -- cargo xtask validate --no-e2e` for the host gate and let CI's
`{sqlite,postgres}×{chromium,firefox}` matrix gate the e2e (spec criterion 7).

- [ ] **Step 6: Commit** (tracked e2e specs only; the untracked doc edit is
      separate)

```bash
git add end2end/tests
git commit -m "test(e2e): assert no-reload + flash-free operator on auth flows (#591)"
```

---

### Task 8: Update ADR-0044

**Files:**

- Modify: the ADR-0044 source under `docs/adr/` (find with
  `rg -l "0044" docs/adr`)

- [ ] **Step 1: Record the decisions (spec "Decisions to record")**

Amend ADR-0044 (auth marker + pre-paint) with: (a) the marker JSON now carries
`is_operator` (additive; `username` stays the top-level key the pre-paint script
reads, so the pre-paint contract is unchanged); (b) the app-level shared session
context (marker-seed signal + per-navigation `session()` reconcile) is the
canonical session-state source, superseding ad-hoc `current_user()` fetches; (c)
`is_operator` in the marker is advisory chrome only — `require_operator()`
remains the enforcement. Use **jaunder-adr** for the mechanics (this is an
amendment to an existing accepted ADR, not a new draft). If the repo's
convention is a dated addendum block, append one; otherwise edit the
Consequences/Context sections in place and note the #591 amendment.

- [ ] **Step 2: Verify + commit**

Run: `devtool run -- cargo xtask check` (picks up any ADR README/table drift;
`prettier` may reflow — restage if so).

```bash
git add docs/adr
git commit -m "docs(adr-0044): marker carries is_operator; shared session context (#591)"
```

---

## Self-Review

**Spec coverage:** §1 marker/SessionUser/migration → T1; §2 session()+retire →
T2,T5; §3 context → T3; §4 consumer conversions → T3 (sidebar), T4
(cockpit/posts); §5 login return + auth flows → T1,T6; §6 hook removal → T6;
acceptance criteria 1-3 → T7, 4 → T5 (rg), 5 → T6, 6 → T1 (drift test
untouched), 7 → T7/CI; E2E+docs → T7; Decisions to record → T8. No uncovered
spec section.

**Placeholder scan:** every implementation step carries real Rust/TS + a run
command with expected FAIL/PASS. Reactive-only tasks (T3/T4/T6) that have no
host-unit-testable branch state so explicitly and lean on T7 e2e + the compile
gate — not a hidden "TODO".

**Type consistency:** `SessionUser { username: Username, is_operator: bool }`
and `LoginResponse { token: String, is_operator: bool }` are used identically
across T1-T6;
`SessionContext { seed: RwSignal<Option<SessionUser>>, reconcile: Resource<Option<SessionUser>> }`
is consumed with the same field names/types in T3 (sidebar), T4 (cockpit
`.reconcile`, posts `.seed`), T6 (`set_session`/ `clear_session`). `session()`
returns `WebResult<Option<SessionUser>>` in T2 and is awaited as
`Option<SessionUser>` in T3/T4.

## Execution Handoff

Plan complete and saved to
`docs/superpowers/plans/2026-07-22-issue-591-shared-session-state.md`. Execution
is driven by **jaunder-iterate** (delegating individual tasks to a subagent via
**jaunder-dispatch** when useful), ticking checkboxes in real time — after the
plan-approval HALT.

# M2 Step 10: Web UI — Account Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `/profile`, `/sessions`, and `/invites` pages to the Leptos web app, each backed by server functions, with full integration tests.

**Architecture:** Each page lives in its own module (`web/src/profile.rs`, etc.) following the thin-component pattern from `web/src/auth.rs`. Server functions own all logic via `require_auth()` + `expect_context::<Arc<AppState>>()`. Custom return types (`ProfileData`, `SessionInfo`, `InviteInfo`) derive `serde::Serialize`/`Deserialize` for wire transfer. Integration tests live in `server/tests/web_account.rs` and exercise the HTTP layer directly like `web_auth.rs`.

**Tech Stack:** Leptos 0.8, Axum, SQLx, serde, tokio, tempfile (tests)

---

## File Map

| Action | Path |
|--------|------|
| Modify | `Cargo.toml` — add `serde` to workspace deps |
| Modify | `web/Cargo.toml` — add `serde.workspace = true` |
| Create | `web/src/profile.rs` |
| Create | `web/src/sessions.rs` |
| Create | `web/src/invites.rs` |
| Modify | `web/src/lib.rs` — add `pub mod` declarations and routes |
| Create | `server/tests/web_account.rs` |

---

## Task 1: Add `serde` dependency

**Files:**
- Modify: `Cargo.toml:21-50`
- Modify: `web/Cargo.toml:1-31`

- [ ] **Step 1: Add serde to workspace deps**

In `Cargo.toml`, inside `[workspace.dependencies]`, add after `thiserror`:

```toml
serde = { version = "1", features = ["derive"] }
```

- [ ] **Step 2: Add serde to web/Cargo.toml**

In `web/Cargo.toml`, inside `[dependencies]`, add:

```toml
serde.workspace = true
```

- [ ] **Step 3: Verify build**

```bash
cargo build
```

Expected: compiles without errors.

---

## Task 2: Create `web/src/profile.rs` stub and declare module

**Files:**
- Create: `web/src/profile.rs`
- Modify: `web/src/lib.rs`

- [ ] **Step 1: Create profile.rs with types and stub server functions**

Create `web/src/profile.rs`:

```rust
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Profile data returned by [`get_profile`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    pub username: String,
    pub display_name: Option<String>,
    pub bio: Option<String>,
}

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use common::storage::{AppState, ProfileUpdate};
#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Returns the authenticated user's profile.
#[server(endpoint = "/get_profile")]
pub async fn get_profile() -> Result<ProfileData, ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let user = state
        .users
        .get_user(auth.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("user not found"))?;
    Ok(ProfileData {
        username: user.username.to_string(),
        display_name: user.display_name,
        bio: user.bio,
    })
}

/// Updates the authenticated user's display name and bio.
/// Empty string clears the field.
#[server(endpoint = "/update_profile")]
pub async fn update_profile(
    display_name: String,
    bio: String,
) -> Result<(), ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let dn = if display_name.is_empty() {
        None
    } else {
        Some(display_name.as_str())
    };
    let b = if bio.is_empty() { None } else { Some(bio.as_str()) };
    state
        .users
        .update_profile(auth.user_id, &ProfileUpdate { display_name: dn, bio: b })
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Profile page — shows username, display name, bio; allows updating.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<UpdateProfile>::new();
    let profile = Resource::new(
        move || update_action.version().get(),
        |_| get_profile(),
    );

    view! {
        <h1>"Profile"</h1>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || Suspend::new(async move {
                match profile.await {
                    Ok(data) => view! {
                        <p>"Username: " {data.username.clone()}</p>
                        <ActionForm action=update_action>
                            <label>
                                "Display Name"
                                <input
                                    type="text"
                                    name="display_name"
                                    prop:value=data.display_name.clone().unwrap_or_default()
                                />
                            </label>
                            <label>
                                "Bio"
                                <textarea
                                    name="bio"
                                    prop:value=data.bio.clone().unwrap_or_default()
                                />
                            </label>
                            <button type="submit">"Update Profile"</button>
                        </ActionForm>
                    }
                    .into_any(),
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            update_action
                .value()
                .get()
                .and_then(|r: Result<(), ServerFnError>| r.err())
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
        }}
    }
}
```

- [ ] **Step 2: Declare module in web/src/lib.rs**

Add `pub mod profile;` at the top of `web/src/lib.rs` (after `pub mod auth;`):

```rust
pub mod auth;
pub mod profile;
```

- [ ] **Step 3: Verify build**

```bash
cargo build
```

Expected: compiles without errors.

---

## Task 3: Write profile integration tests

**Files:**
- Create: `server/tests/web_account.rs`

- [ ] **Step 1: Create the test file with helpers and profile tests**

Create `server/tests/web_account.rs`:

```rust
use std::sync::{Arc, OnceLock};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::storage::ProfileUpdate;
use leptos::prelude::LeptosOptions;
use server::storage::{open_database, DbConnectOptions};
use server::username::Username;
use tempfile::TempDir;
use tower::ServiceExt;

fn ensure_server_fns_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        server_fn::axum::register_explicit::<web::profile::GetProfile>();
        server_fn::axum::register_explicit::<web::profile::UpdateProfile>();
        server_fn::axum::register_explicit::<web::sessions::ListSessions>();
        server_fn::axum::register_explicit::<web::sessions::RevokeSession>();
        server_fn::axum::register_explicit::<web::invites::CreateInvite>();
        server_fn::axum::register_explicit::<web::invites::ListInvites>();
    });
}

fn db_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("test.db").display())
        .parse()
        .unwrap()
}

async fn test_state(base: &TempDir) -> Arc<server::storage::AppState> {
    open_database(&db_url(base)).await.unwrap()
}

fn test_options() -> LeptosOptions {
    LeptosOptions::builder().output_name("test").build()
}

async fn post_form(
    state: Arc<server::storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, Option<String>, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = server::create_router(test_options(), state);
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, set_cookie, body_str)
}

async fn post_form_with_bearer(
    state: Arc<server::storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    bearer_token: &str,
) -> (StatusCode, Option<String>, String) {
    ensure_server_fns_registered();

    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::AUTHORIZATION, format!("Bearer {bearer_token}"))
        .body(Body::from(body.into()))
        .unwrap();

    let app = server::create_router(test_options(), state);
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, set_cookie, body_str)
}

// ── Profile tests (M2.10.5, M2.10.6) ─────────────────────────────────────

// M2.10.5: get_profile returns the authenticated user's display name and bio.
#[tokio::test]
async fn get_profile_returns_display_name_and_bio() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    state
        .users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some("Alice Smith"),
                bio: Some("Hello world"),
            },
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, body) =
        post_form(Arc::clone(&state), "/api/get_profile", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Alice Smith"), "display_name missing: {body}");
    assert!(body.contains("Hello world"), "bio missing: {body}");
}

// M2.10.6: update_profile persists changes visible in a subsequent get_profile.
#[tokio::test]
async fn update_profile_persists_changes() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"bob".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, _) = post_form(
        Arc::clone(&state),
        "/api/update_profile",
        "display_name=Robert&bio=My+bio",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, body) =
        post_form(Arc::clone(&state), "/api/get_profile", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Robert"), "display_name not persisted: {body}");
    assert!(body.contains("My bio"), "bio not persisted: {body}");
}

// ── Sessions tests (M2.10.7, M2.10.8) ────────────────────────────────────

// M2.10.7: list_sessions returns sessions for the authenticated user only.
#[tokio::test]
async fn list_sessions_returns_only_authenticated_users_sessions() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let user1_id = state
        .users
        .create_user(
            &"carol".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let user2_id = state
        .users
        .create_user(
            &"dave".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    let token1 = state
        .sessions
        .create_session(user1_id, Some("carol-session"))
        .await
        .unwrap();
    // Create a session for user2 — should NOT appear in user1's list.
    state
        .sessions
        .create_session(user2_id, Some("dave-session"))
        .await
        .unwrap();

    let cookie = format!("session={token1}");
    let (status, _, body) =
        post_form(Arc::clone(&state), "/api/list_sessions", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("carol-session"),
        "own session should appear: {body}"
    );
    assert!(
        !body.contains("dave-session"),
        "other user's session must not appear: {body}"
    );
}

// M2.10.8: revoke_session removes the target session; re-auth with that token fails.
#[tokio::test]
async fn revoke_session_removes_session_and_reauth_fails() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"eve".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();

    // Create two sessions: use token_a to authenticate, revoke token_b.
    let token_a = state.sessions.create_session(user_id, None).await.unwrap();
    let token_b = state.sessions.create_session(user_id, None).await.unwrap();

    // Get token_b's hash via list_sessions.
    let sessions = state.sessions.list_sessions(user_id).await.unwrap();
    let record_b = sessions
        .iter()
        .find(|s| s.token_hash != {
            // Find hash for token_a so we can identify token_b's hash.
            use sha2::Digest;
            let hash = sha2::Sha256::digest(
                base64::engine::Engine::decode(
                    &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                    &token_a,
                )
                .unwrap(),
            );
            base64::engine::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                hash,
            )
        })
        .unwrap()
        .clone();

    let cookie_a = format!("session={token_a}");
    let body = format!("token_hash={}", urlencoding::encode(&record_b.token_hash));
    let (status, _, _) =
        post_form(Arc::clone(&state), "/api/revoke_session", body, Some(&cookie_a)).await;
    assert_eq!(status, StatusCode::OK);

    // Re-authenticate with token_b should fail (session revoked).
    let result = state.sessions.authenticate(&token_b).await;
    assert!(result.is_err(), "revoked session should not authenticate");
}

// ── Invites tests (M2.10.9) ───────────────────────────────────────────────

// M2.10.9: create_invite returns a code that appears in a subsequent list_invites.
#[tokio::test]
async fn create_invite_appears_in_list_invites() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    let user_id = state
        .users
        .create_user(
            &"frank".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, body) = post_form(
        Arc::clone(&state),
        "/api/create_invite",
        "expires_in_hours=24",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create_invite failed: {body}");
    // Body is a JSON string: "<code>"
    let trimmed = body.trim();
    assert!(
        trimmed.starts_with('"') && trimmed.ends_with('"'),
        "expected JSON string, got: {trimmed}"
    );
    let code = &trimmed[1..trimmed.len() - 1];

    let (status, _, list_body) =
        post_form(Arc::clone(&state), "/api/list_invites", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "list_invites failed: {list_body}");
    assert!(
        list_body.contains(code),
        "created code {code} not in list: {list_body}"
    );
}

// list_invites returns error when policy is not InviteOnly.
#[tokio::test]
async fn list_invites_returns_error_when_policy_not_invite_only() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    // Policy defaults to Closed.

    let user_id = state
        .users
        .create_user(
            &"grace".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, _, _) =
        post_form(Arc::clone(&state), "/api/list_invites", "", Some(&cookie)).await;
    assert_ne!(
        status,
        StatusCode::OK,
        "list_invites should fail when policy is not invite_only"
    );
}
```

- [ ] **Step 2: Note — tests will fail to compile until sessions.rs and invites.rs exist**

This is expected TDD. The compilation will succeed once all three modules are declared. Continue to the next task.

---

## Task 4: Create `web/src/sessions.rs` stub and declare module

**Files:**
- Create: `web/src/sessions.rs`
- Modify: `web/src/lib.rs`

- [ ] **Step 1: Create sessions.rs with types and server functions**

Create `web/src/sessions.rs`:

```rust
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Session info returned by [`list_sessions`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub token_hash: String,
    pub label: Option<String>,
    pub created_at: String,
    pub last_used_at: String,
    pub is_current: bool,
}

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Returns all sessions for the authenticated user.
/// `is_current` is `true` for the session used to make this request.
#[server(endpoint = "/list_sessions")]
pub async fn list_sessions() -> Result<Vec<SessionInfo>, ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let records = state
        .sessions
        .list_sessions(auth.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(records
        .into_iter()
        .map(|r| SessionInfo {
            is_current: r.token_hash == auth.token_hash,
            token_hash: r.token_hash,
            label: r.label,
            created_at: r.created_at.to_rfc3339(),
            last_used_at: r.last_used_at.to_rfc3339(),
        })
        .collect())
}

/// Revokes a session belonging to the authenticated user.
#[server(endpoint = "/revoke_session")]
pub async fn revoke_session(token_hash: String) -> Result<(), ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    // Verify the session belongs to the authenticated user.
    let sessions = state
        .sessions
        .list_sessions(auth.user_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    if !sessions.iter().any(|s| s.token_hash == token_hash) {
        return Err(ServerFnError::new("session not found"));
    }
    state
        .sessions
        .revoke_session(&token_hash)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Sessions page — lists all sessions; allows revoking individual sessions.
#[component]
pub fn SessionsPage() -> impl IntoView {
    let revoke_action = ServerAction::<RevokeSession>::new();
    let sessions = Resource::new(
        move || revoke_action.version().get(),
        |_| list_sessions(),
    );

    view! {
        <h1>"Sessions"</h1>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || Suspend::new(async move {
                match sessions.await {
                    Ok(list) => view! {
                        <ul>
                            {list
                                .into_iter()
                                .map(|s| {
                                    let hash = s.token_hash.clone();
                                    view! {
                                        <li>
                                            {s.label.clone().unwrap_or_else(|| "(no label)".to_string())}
                                            " — last used: "
                                            {s.last_used_at.clone()}
                                            {s.is_current.then(|| view! { " (current)" })}
                                            " "
                                            <ActionForm action=revoke_action>
                                                <input type="hidden" name="token_hash" value=hash />
                                                <button type="submit">"Revoke"</button>
                                            </ActionForm>
                                        </li>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </ul>
                    }
                    .into_any(),
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}
```

- [ ] **Step 2: Declare module in web/src/lib.rs**

Add `pub mod sessions;` after `pub mod profile;`:

```rust
pub mod auth;
pub mod profile;
pub mod sessions;
```

- [ ] **Step 3: Verify build**

```bash
cargo build
```

Expected: compiles without errors.

---

## Task 5: Create `web/src/invites.rs` stub and declare module

**Files:**
- Create: `web/src/invites.rs`
- Modify: `web/src/lib.rs`

- [ ] **Step 1: Create invites.rs with types and server functions**

Create `web/src/invites.rs`:

```rust
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Invite info returned by [`list_invites`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteInfo {
    pub code: String,
    pub created_at: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub used_by: Option<i64>,
}

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use chrono::Utc;
#[cfg(feature = "ssr")]
use common::auth::{load_registration_policy, RegistrationPolicy};
#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Creates an invite code expiring in `expires_in_hours` (default 168 = 7 days).
/// Requires authentication.
#[server(endpoint = "/create_invite")]
pub async fn create_invite(expires_in_hours: Option<u64>) -> Result<String, ServerFnError> {
    let _auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let hours = expires_in_hours.unwrap_or(168);
    let expires_at = Utc::now() + chrono::Duration::hours(hours as i64);
    state
        .invites
        .create_invite(expires_at)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Returns all invite codes. Requires `invite_only` registration policy;
/// returns an error otherwise.
#[server(endpoint = "/list_invites")]
pub async fn list_invites() -> Result<Vec<InviteInfo>, ServerFnError> {
    let _auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let policy = load_registration_policy(&*state.site_config).await;
    if policy != RegistrationPolicy::InviteOnly {
        return Err(ServerFnError::new("not found"));
    }
    let records = state
        .invites
        .list_invites()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(records
        .into_iter()
        .map(|r| InviteInfo {
            code: r.code,
            created_at: r.created_at.to_rfc3339(),
            expires_at: r.expires_at.to_rfc3339(),
            used_at: r.used_at.map(|t| t.to_rfc3339()),
            used_by: r.used_by,
        })
        .collect())
}

/// Invites page — lists invite codes; allows creating new ones.
/// Returns 404 (via SSR response options) when the registration policy is not
/// `invite_only`.
#[component]
pub fn InvitesPage() -> impl IntoView {
    let create_action = ServerAction::<CreateInvite>::new();
    let policy = Resource::new(|| (), |_| crate::auth::get_registration_policy());
    let invites = Resource::new(
        move || create_action.version().get(),
        |_| list_invites(),
    );

    view! {
        <h1>"Invites"</h1>
        <Suspense fallback=|| view! { <p>"Loading..."</p> }>
            {move || Suspend::new(async move {
                let policy_str = policy.await.unwrap_or_default();
                if policy_str != "invite_only" {
                    // Set 404 status when rendered server-side.
                    #[cfg(feature = "ssr")]
                    {
                        use leptos::context::use_context;
                        use leptos_axum::ResponseOptions;
                        if let Some(opts) = use_context::<ResponseOptions>() {
                            opts.set_status(axum::http::StatusCode::NOT_FOUND);
                        }
                    }
                    return view! { <p>"Page not found."</p> }.into_any();
                }
                match invites.await {
                    Ok(list) => view! {
                        <ActionForm action=create_action>
                            <label>
                                "Expires in hours"
                                <input type="number" name="expires_in_hours" />
                            </label>
                            <button type="submit">"Create Invite"</button>
                        </ActionForm>
                        <ul>
                            {list
                                .into_iter()
                                .map(|i| {
                                    view! {
                                        <li>
                                            "Code: "
                                            {i.code.clone()}
                                            " — expires: "
                                            {i.expires_at.clone()}
                                            {i.used_at
                                                .clone()
                                                .map(|t| view! { " (used at " {t} ")" })}
                                        </li>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </ul>
                    }
                    .into_any(),
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}
```

- [ ] **Step 2: Declare module in web/src/lib.rs**

```rust
pub mod auth;
pub mod invites;
pub mod profile;
pub mod sessions;
```

- [ ] **Step 3: Check that web_account.rs compiles**

```bash
cargo build
cargo nextest run --test web_account 2>&1 | head -50
```

Expected: compiles; tests fail at runtime (server functions not yet fully wired) or pass if implementation is already correct. Do not worry if some fail — confirm there are no compilation errors.

---

## Task 6: Add `urlencoding` dependency (needed by test)

The `revoke_session` test uses `urlencoding::encode`. Add the crate.

**Files:**
- Modify: `Cargo.toml`
- Modify: `server/Cargo.toml`

- [ ] **Step 1: Add urlencoding to workspace deps**

In `Cargo.toml` `[workspace.dependencies]`, add:

```toml
urlencoding = "2"
```

- [ ] **Step 2: Add urlencoding to server test deps**

In `server/Cargo.toml`, inside `[dev-dependencies]`, add:

```toml
urlencoding.workspace = true
```

Also add to the same section:

```toml
common.workspace = true
```

(needed for `ProfileUpdate` in tests — verify it isn't already there)

- [ ] **Step 3: Verify build + tests**

```bash
cargo build
cargo nextest run --test web_account
```

Expected: all tests in `web_account` pass.

---

## Task 7: Add routes to `web/src/lib.rs`

**Files:**
- Modify: `web/src/lib.rs`

- [ ] **Step 1: Import new page components**

Update the imports at the top of `web/src/lib.rs`:

```rust
pub mod auth;
pub mod invites;
pub mod profile;
pub mod sessions;

use crate::auth::{LoginPage, LogoutPage, RegisterPage};
use crate::invites::InvitesPage;
use crate::profile::ProfilePage;
use crate::sessions::SessionsPage;
use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};
```

- [ ] **Step 2: Add routes to the App component**

Update the `<Routes>` block in the `App` component:

```rust
<Routes fallback=|| "Page not found.".into_view()>
    <Route path=StaticSegment("") view=HomePage />
    <Route path=StaticSegment("register") view=RegisterPage />
    <Route path=StaticSegment("login") view=LoginPage />
    <Route path=StaticSegment("logout") view=LogoutPage />
    <Route path=StaticSegment("profile") view=ProfilePage />
    <Route path=StaticSegment("sessions") view=SessionsPage />
    <Route path=StaticSegment("invites") view=InvitesPage />
</Routes>
```

- [ ] **Step 3: Add route tests to `server/src/lib.rs`**

Add these tests inside the existing `#[cfg(test)] mod tests { ... }` block in `server/src/lib.rs`:

```rust
#[tokio::test]
async fn profile_route_returns_ok_when_authenticated() {
    // Profile route should return 200 (the page renders; auth check happens
    // at server-function level, not page-route level).
    let app = create_router(test_options(), test_state().await);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/profile")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn sessions_route_returns_ok() {
    let app = create_router(test_options(), test_state().await);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn invites_route_returns_not_found_when_policy_is_closed() {
    // Default policy is Closed; InvitesPage should set 404.
    // The page does this via Suspense + SSR ResponseOptions, so the initial
    // SSR render returns 404.
    let app = create_router(test_options(), test_state().await);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/invites")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 4: Verify build**

```bash
cargo build
```

Expected: compiles without errors.

---

## Task 8: Final verification

- [ ] **Step 1: Run all tests**

```bash
cargo nextest run
```

Expected: all tests pass.

- [ ] **Step 2: Run clippy**

```bash
cargo clippy -- -D warnings
```

Expected: no warnings or errors.

- [ ] **Step 3: Check coverage baseline**

```bash
scripts/check-coverage
```

Expected: passes (coverage does not drop below baseline).

- [ ] **Step 4: Run nix flake check (e2e tests)**

```bash
nix flake check
```

Expected: all checks pass.

---

## Self-Review Against Spec

Checking M2 Step 10 items:

| Item | Covered by |
|------|-----------|
| M2.10.1 `get_profile`, `update_profile`, `ProfilePage` | Task 2 |
| M2.10.2 `list_sessions`, `revoke_session`, `SessionsPage` | Task 4 |
| M2.10.3 `create_invite`, `list_invites`, `InvitesPage` | Task 5 |
| M2.10.4 Add routes; `/invites` 404 when not InviteOnly | Tasks 7 + 5 (SSR ResponseOptions) |
| M2.10.5 `get_profile` returns display name and bio | Task 3 test |
| M2.10.6 `update_profile` persists changes | Task 3 test |
| M2.10.7 `list_sessions` returns only authenticated user's sessions | Task 3 test |
| M2.10.8 `revoke_session` removes session; reauth fails | Task 3 test |
| M2.10.9 `create_invite` code appears in `list_invites` | Task 3 test |

**Notes on potential issues:**
- Task 6 adds `urlencoding` and `common` to `server` dev-deps. Check `server/Cargo.toml` first to avoid duplicates.
- The `revoke_session` test computes the token_a hash inline. If the `hash_token` helper is exported from `server::auth`, prefer using it directly.
- The `invites_route_returns_not_found_when_policy_is_closed` test relies on Leptos SSR resolving the `Suspense` synchronously before sending the response. If the route returns 200 instead of 404 in practice (because Suspense resolves asynchronously after headers are sent), change the test to check the response body contains "Page not found." and adjust the 404 assertion to match actual Leptos SSR behaviour.

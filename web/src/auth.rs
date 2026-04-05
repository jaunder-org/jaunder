#[cfg(feature = "ssr")]
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
#[cfg(feature = "ssr")]
use common::auth::{load_registration_policy, RegistrationPolicy};
#[cfg(feature = "ssr")]
use common::password::Password;
#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use common::username::Username;
#[cfg(feature = "ssr")]
use std::sync::Arc;

// ---------------------------------------------------------------------------
// AuthUser
// ---------------------------------------------------------------------------

/// The authenticated user extracted from a valid session cookie or Bearer token.
#[cfg(feature = "ssr")]
pub struct AuthUser {
    pub user_id: i64,
    pub username: Username,
    pub token_hash: String,
}

#[cfg(feature = "ssr")]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Cookie takes precedence over Authorization header.
        let raw_token = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                s.split(';')
                    .map(str::trim)
                    .find(|c| c.starts_with("session="))
                    .and_then(|c| c.strip_prefix("session="))
                    .map(str::to_string)
            })
            .or_else(|| {
                parts
                    .headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .map(str::to_string)
            });

        let raw_token = raw_token.ok_or(StatusCode::UNAUTHORIZED)?;

        let state = parts
            .extensions
            .get::<Arc<AppState>>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        match state.sessions.authenticate(&raw_token).await {
            Ok(record) => Ok(AuthUser {
                user_id: record.user_id,
                username: record.username,
                token_hash: record.token_hash,
            }),
            Err(_) => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

/// Extracts the authenticated user inside a Leptos server function.
/// Returns `ServerFnError` when no valid session is present.
#[cfg(feature = "ssr")]
pub async fn require_auth() -> Result<AuthUser, leptos::prelude::ServerFnError> {
    leptos_axum::extract::<AuthUser>()
        .await
        .map_err(|_| leptos::prelude::ServerFnError::new("unauthorized"))
}

// ---------------------------------------------------------------------------
// Cookie helpers
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
fn set_session_cookie(raw_token: &str) {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;
    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = format!(
            "session={}; HttpOnly; SameSite=Lax; Path=/; Secure",
            raw_token
        );
        if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

#[cfg(feature = "ssr")]
fn clear_session_cookie() {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;
    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = "session=; HttpOnly; SameSite=Lax; Path=/; Secure; Max-Age=0";
        if let Ok(val) = axum::http::HeaderValue::from_str(cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

// ---------------------------------------------------------------------------
// Server functions
// ---------------------------------------------------------------------------

use leptos::prelude::*;

/// Returns the site's current registration policy as a string.
/// Possible values: `"open"`, `"invite_only"`, `"closed"`.
#[server(endpoint = "/get_registration_policy")]
pub async fn get_registration_policy() -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let policy = load_registration_policy(&*state.site_config).await;
    Ok(policy.to_string())
}

/// Registers a new user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/register")]
pub async fn register(
    username: String,
    password: String,
    invite_code: Option<String>,
) -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let username = username
        .to_lowercase()
        .parse::<Username>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let password = password
        .parse::<Password>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let policy = load_registration_policy(&*state.site_config).await;

    let user_id = match policy {
        RegistrationPolicy::Open => state
            .users
            .create_user(&username, &password, None)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?,
        RegistrationPolicy::InviteOnly => {
            let code = invite_code
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ServerFnError::new("invite code required"))?;
            state
                .atomic
                .create_user_with_invite(&username, &password, None, &code)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?
        }
        RegistrationPolicy::Closed => {
            return Err(ServerFnError::new("registration is closed"));
        }
    };

    let raw_token = state
        .sessions
        .create_session(user_id, None)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    set_session_cookie(&raw_token);
    Ok(raw_token)
}

/// Authenticates a user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/login")]
pub async fn login(
    username: String,
    password: String,
    label: Option<String>,
) -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let username = username
        .to_lowercase()
        .parse::<Username>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let password = password
        .parse::<Password>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let record = state
        .users
        .authenticate(&username, &password)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let label = label.filter(|s| !s.is_empty());
    let raw_token = state
        .sessions
        .create_session(record.user_id, label.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    set_session_cookie(&raw_token);
    Ok(raw_token)
}

/// Revokes the current session and clears the `session` cookie.
#[server(endpoint = "/logout")]
pub async fn logout() -> Result<(), ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    state
        .sessions
        .revoke_session(&auth.token_hash)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    clear_session_cookie();
    Ok(())
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Registration page.
#[component]
pub fn RegisterPage() -> impl IntoView {
    let register_action = ServerAction::<Register>::new();
    let policy = Resource::new(|| (), |_| get_registration_policy());
    let username = RwSignal::new(String::new());

    view! {
        <h1>"Register"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                let p = policy.await;
                let is_invite_only = p.as_deref() == Ok("invite_only");
                view! {
                    <ActionForm action=register_action>
                        <label>
                            "Username"
                            <input
                                type="text"
                                name="username"
                                prop:value=username
                                on:input=move |ev| {
                                    username.set(event_target_value(&ev).to_lowercase());
                                }
                            />
                        </label>
                        <label>"Password" <input type="password" name="password" /></label>
                        {is_invite_only
                            .then(|| {
                                view! {
                                    <label>
                                        "Invite code" <input type="text" name="invite_code" />
                                    </label>
                                }
                            })}
                        <button type="submit">"Register"</button>
                    </ActionForm>
                }
            })}
        </Suspense>
        {move || {
            register_action
                .value()
                .get()
                .and_then(|r: Result<String, ServerFnError>| r.err())
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
        }}
    }
}

/// Login page.
#[component]
pub fn LoginPage() -> impl IntoView {
    let login_action = ServerAction::<Login>::new();
    let username = RwSignal::new(String::new());

    view! {
        <h1>"Login"</h1>
        <ActionForm action=login_action>
            <label>
                "Username"
                <input
                    type="text"
                    name="username"
                    prop:value=username
                    on:input=move |ev| {
                        username.set(event_target_value(&ev).to_lowercase());
                    }
                />
            </label>
            <label>"Password" <input type="password" name="password" /></label>
            <button type="submit">"Login"</button>
        </ActionForm>
        {move || {
            login_action
                .value()
                .get()
                .and_then(|r: Result<String, ServerFnError>| r.err())
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
        }}
    }
}

/// Logout page — fires the logout server action on mount.
#[component]
pub fn LogoutPage() -> impl IntoView {
    let logout_action = ServerAction::<Logout>::new();

    Effect::new(move |_| {
        logout_action.dispatch(Logout {});
    });

    view! {
        <h1>"Logging out\u{2026}"</h1>
        {move || {
            logout_action
                .value()
                .get()
                .map(|r: Result<(), ServerFnError>| {
                    match r {
                        Ok(_) => view! { <p>"You have been logged out."</p> }.into_any(),
                        Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                    }
                })
        }}
    }
}

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
    let sessions = Resource::new(move || revoke_action.version().get(), |_| list_sessions());

    view! {
        <h1>"Sessions"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match sessions.await {
                    Ok(list) => {
                        view! {
                            <ul>
                                {list
                                    .into_iter()
                                    .map(|s| {
                                        let hash = s.token_hash.clone();
                                        view! {
                                            <li>
                                                {s
                                                    .label
                                                    .clone()
                                                    .unwrap_or_else(|| "(no label)".to_string())}
                                                " — last used: " {s.last_used_at.clone()}
                                                {s.is_current.then_some(view! { " (current)" })} " "
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
                            .into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

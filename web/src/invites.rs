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
    let duration = i64::try_from(hours)
        .ok()
        .and_then(chrono::Duration::try_hours)
        .ok_or_else(|| ServerFnError::new("expires_in_hours too large"))?;
    let expires_at = Utc::now() + duration;
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
    let invites = Resource::new(move || create_action.version().get(), |_| list_invites());

    view! {
        <h1>"Invites"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                let policy_str = policy.await.unwrap_or_default();
                if policy_str != "invite_only" {
                    #[cfg(feature = "ssr")]
                    {
                        use leptos::context::use_context;
                        use leptos_axum::ResponseOptions;
                        if let Some(opts) = use_context::<ResponseOptions>() {
                            opts.set_status(axum::http::StatusCode::NOT_FOUND);
                        }
                    }
                    return // Set 404 status when rendered server-side.
                    view! { <p>"Page not found."</p> }
                        .into_any();
                }
                match invites.await {
                    Ok(list) => {
                        view! {
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
                                                "Code: " {i.code.clone()} " — expires: "
                                                {i.expires_at.clone()}
                                                {i
                                                    .used_at
                                                    .clone()
                                                    .map(|t| {
                                                        view! {
                                                            " (used at "
                                                            {t}
                                                            ")"
                                                        }
                                                    })}
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

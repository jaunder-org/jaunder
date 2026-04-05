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
pub async fn update_profile(display_name: String, bio: String) -> Result<(), ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let dn = if display_name.is_empty() {
        None
    } else {
        Some(display_name.as_str())
    };
    let b = if bio.is_empty() {
        None
    } else {
        Some(bio.as_str())
    };
    state
        .users
        .update_profile(
            auth.user_id,
            &ProfileUpdate {
                display_name: dn,
                bio: b,
            },
        )
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Profile page — shows username, display name, bio; allows updating.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<UpdateProfile>::new();
    let profile = Resource::new(move || update_action.version().get(), |_| get_profile());

    view! {
        <h1>"Profile"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match profile.await {
                    Ok(data) => {
                        view! {
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
                            .into_any()
                    }
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

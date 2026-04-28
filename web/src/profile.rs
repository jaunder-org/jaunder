use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
use crate::error::InternalError;
use crate::error::WebResult;

/// Profile data returned by [`get_profile`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    pub username: String,
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use common::storage::{AppState, ProfileUpdate};
#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Returns the authenticated user's profile.
#[server(endpoint = "/get_profile")]
pub async fn get_profile() -> WebResult<ProfileData> {
    crate::web_server_fn!("get_profile", => {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();
        let user = state
            .users
            .get_user(auth.user_id)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("user"))?;
        Ok(ProfileData {
            username: user.username.to_string(),
            display_name: user.display_name,
            bio: user.bio,
            email: user.email.map(|e| e.to_string()),
            email_verified: user.email_verified,
        })
    })
}

/// Updates the authenticated user's display name and bio.
/// Empty string clears the field.
#[server(endpoint = "/update_profile")]
pub async fn update_profile(display_name: String, bio: String) -> WebResult<()> {
    crate::web_server_fn!("update_profile", display_name, bio => {
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
            .map_err(InternalError::storage)
    })
}

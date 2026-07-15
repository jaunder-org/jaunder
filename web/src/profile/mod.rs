// Shared imports (no cfg needed)
use crate::error::WebResult;
use common::display_name::DisplayName;
use common::email::Email;
use common::username::Username;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

// All server-only imports in one place
#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    std::sync::Arc,
    storage::{
        get_default_post_format as storage_get_default_post_format,
        set_default_post_format as storage_set_default_post_format, PostFormat, ProfileUpdate,
        UserConfigStorage, UserStorage,
    },
};

/// Profile data returned by [`get_profile`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    pub username: Username,
    pub display_name: Option<DisplayName>,
    pub bio: Option<String>,
    pub email: Option<Email>,
    pub email_verified: bool,
}

/// Returns the authenticated user's profile.
#[server(endpoint = "/get_profile")]
pub async fn get_profile() -> WebResult<ProfileData> {
    boundary!("get_profile", {
        let auth = require_auth().await?;
        let users = expect_context::<Arc<dyn UserStorage>>();
        let user = users
            .get_user(auth.user_id)
            .await?
            .ok_or_else(|| InternalError::not_found("user"))?;
        Ok(ProfileData {
            username: user.username,
            display_name: user.display_name,
            bio: user.bio,
            email: user.email,
            email_verified: user.email_verified,
        })
    })
}

/// Updates the authenticated user's display name and bio.
///
/// `display_name` is a typed [`DisplayName`] wire arg pre-validated on the client
/// (ADR-0065): `None` clears it, `Some` is already trimmed/bounded. `bio` is
/// free-form; blank (empty or whitespace-only) clears it.
#[server(endpoint = "/update_profile")]
pub async fn update_profile(display_name: Option<DisplayName>, bio: String) -> WebResult<()> {
    boundary!("update_profile", {
        let auth = require_auth().await?;
        let users = expect_context::<Arc<dyn UserStorage>>();
        let b = common::text::non_empty(&bio);
        users
            .update_profile(
                auth.user_id,
                &ProfileUpdate {
                    display_name: display_name.as_ref(),
                    bio: b,
                },
            )
            .await
            .map_err(InternalError::storage)
    })
}

/// Retrieves the authenticated user's default post format preference.
#[server(endpoint = "/get_default_post_format")]
pub async fn get_default_post_format() -> WebResult<String> {
    boundary!("get_default_post_format", {
        let auth = require_auth().await?;
        let config = expect_context::<Arc<dyn UserConfigStorage>>();
        let format = storage_get_default_post_format(config.as_ref(), auth.user_id).await?;
        Ok(format.to_string())
    })
}

/// Sets the authenticated user's default post format preference.
#[server(endpoint = "/set_default_post_format")]
pub async fn set_default_post_format(format: String) -> WebResult<()> {
    boundary!("set_default_post_format", {
        let auth = require_auth().await?;
        let config = expect_context::<Arc<dyn UserConfigStorage>>();
        let post_format = format.parse::<PostFormat>()?;
        storage_set_default_post_format(config.as_ref(), auth.user_id, post_format).await?;
        Ok(())
    })
}

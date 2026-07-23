//! Profile wire DTOs + `#[server]` endpoints (ADR-0070, amended #530): the
//! `ProfileData` payload and the `get_profile` / `update_profile` /
//! `get_default_post_format` / `set_default_post_format` server fns. Dual-compiled
//! (host + wasm); the vertical's one grouped `#[cfg(feature = "server")]` use-block
//! lives here. Re-exported from `mod.rs` so `crate::profile::…` paths stay stable.

// Shared imports (no cfg needed)
use crate::error::WebResult;
use common::bio::Bio;
use common::display_name::DisplayName;
use common::email::Email;
use common::render::PostFormat;
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
        set_default_post_format as storage_set_default_post_format, ProfileUpdate,
        UserConfigStorage, UserStorage,
    },
};

/// Profile data returned by [`get_profile`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileData {
    pub username: Username,
    pub display_name: Option<DisplayName>,
    pub bio: Option<Bio>,
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
/// `display_name` and `bio` are typed wire args pre-validated on the client
/// (ADR-0065): `None` clears (the field is omitted), `Some` is already
/// trimmed/bounded. Both `Option`s model presence, so no `non_empty` shim is
/// needed — an empty wire value is rejected at decode, clearing goes via omission.
#[server(endpoint = "/update_profile")]
pub async fn update_profile(display_name: Option<DisplayName>, bio: Option<Bio>) -> WebResult<()> {
    boundary!("update_profile", {
        let auth = require_auth().await?;
        let users = expect_context::<Arc<dyn UserStorage>>();
        users
            .update_profile(
                auth.user_id,
                &ProfileUpdate {
                    display_name: display_name.as_ref(),
                    bio: bio.as_ref(),
                },
            )
            .await
            .map_err(InternalError::storage)
    })
}

/// Retrieves the authenticated user's default post format preference.
#[server(endpoint = "/get_default_post_format")]
pub async fn get_default_post_format() -> WebResult<PostFormat> {
    boundary!("get_default_post_format", {
        let auth = require_auth().await?;
        let config = expect_context::<Arc<dyn UserConfigStorage>>();
        let format = storage_get_default_post_format(config.as_ref(), auth.user_id).await?;
        Ok(format)
    })
}

/// Sets the authenticated user's default post format preference.
#[server(endpoint = "/set_default_post_format")]
pub async fn set_default_post_format(format: PostFormat) -> WebResult<()> {
    boundary!("set_default_post_format", {
        let auth = require_auth().await?;
        let config = expect_context::<Arc<dyn UserConfigStorage>>();
        storage_set_default_post_format(config.as_ref(), auth.user_id, format).await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::SetDefaultPostFormat;
    use common::render::PostFormat;

    #[test]
    fn set_default_post_format_wire_rejects_unknown_token() {
        // The profile control submits `format=<token>` via an <ActionForm>, decoded
        // through server_fn's default Url codec (serde_qs). A valid token decodes; a
        // bogus one is rejected at the wire boundary once the arg is a typed PostFormat.
        let ok: SetDefaultPostFormat = serde_qs::from_str("format=markdown").unwrap();
        assert_eq!(ok.format, PostFormat::Markdown);
        assert!(serde_qs::from_str::<SetDefaultPostFormat>("format=bogus").is_err());
    }
}

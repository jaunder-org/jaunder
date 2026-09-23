//! Profile wire DTOs and authenticated `#[server]` endpoints.
//!
//! The profile owns author-scoped settings because the authenticated User is the
//! publication. Site-wide settings remain under the operator-only `site`
//! vertical. Dual-compiled (host + wasm); the vertical's grouped server imports
//! live here.

use crate::error::WebResult;
use common::{
    MutationOutcome, bio::Bio, content_license::ContentLicense, display_name::DisplayName,
    email::Email, render::PostFormat, username::Username, visibility::DefaultAudience,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth,
    crate::error::{InternalError, from_write_scope_error},
    common::time::UtcInstant,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{
        FeedEventStorage, PostStorage, ProfileUpdate, SiteConfigStorage, UserConfigStorage,
        UserStorage, WriteScope,
    },
};

/// Profile data returned by [`get`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Data {
    pub username: Username,
    pub display_name: Option<DisplayName>,
    pub bio: Option<Bio>,
    pub email: Option<Email>,
    pub email_verified: bool,
}

/// The User's stored override together with the Site value it may inherit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DefaultAudiencePreference {
    pub audience: Option<DefaultAudience>,
    pub site_audience: DefaultAudience,
}

/// Returns the authenticated user's profile.
#[macros::server]
pub async fn get() -> WebResult<Data> {
    let auth = auth::require_auth().await?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    let user = users
        .get_user(auth.user_id)
        .await?
        .ok_or_else(|| InternalError::not_found("user"))?;
    Ok(Data {
        username: user.username,
        display_name: user.display_name,
        bio: user.bio,
        email: user.email,
        email_verified: user.email_verified.is_verified(),
    })
}

/// Updates the authenticated user's display name and bio.
///
/// `display_name` and `bio` are typed wire args pre-validated on the client
/// (ADR-0065): `None` clears (the field is omitted), `Some` is already
/// trimmed/bounded. Both `Option`s model presence, so no `non_empty` shim is
/// needed — an empty wire value is rejected at decode, clearing goes via omission.
#[macros::server(skip_all)]
pub async fn update(
    display_name: Option<DisplayName>,
    bio: Option<Bio>,
) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let users = expect_context::<Arc<dyn UserStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    let now = UtcInstant::now();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                storage::update_profile_with_feed_events(
                    transaction,
                    users.as_ref(),
                    posts.as_ref(),
                    feed_events.as_ref(),
                    auth.user_id,
                    &ProfileUpdate {
                        display_name: display_name.as_ref(),
                        bio: bio.as_ref(),
                    },
                    now,
                )
                .await
                .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Retrieves the authenticated User's audience preference and inherited value.
#[macros::server]
pub async fn get_default_audience() -> WebResult<DefaultAudiencePreference> {
    let auth = auth::require_auth().await?;
    let user_config = expect_context::<Arc<dyn UserConfigStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    Ok(DefaultAudiencePreference {
        audience: storage::get_user_default_audience(user_config.as_ref(), auth.user_id).await?,
        site_audience: site_config.get_default_audience().await?,
    })
}

/// Sets or clears the authenticated User's audience preference.
#[macros::server(skip_all)]
pub async fn set_default_audience(
    audience: Option<DefaultAudience>,
) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let user_config = expect_context::<Arc<dyn UserConfigStorage>>();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                storage::set_user_default_audience(
                    user_config.as_ref(),
                    transaction,
                    auth.user_id,
                    audience,
                )
                .await
                .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Retrieves the authenticated user's default post format preference.
#[macros::server]
pub async fn get_default_post_format() -> WebResult<PostFormat> {
    let auth = auth::require_auth().await?;
    let config = expect_context::<Arc<dyn UserConfigStorage>>();
    let format = storage::get_default_post_format(config.as_ref(), auth.user_id).await?;
    Ok(format)
}

/// Retrieves the authenticated User's current Content License.
#[macros::server]
pub async fn get_content_license() -> WebResult<ContentLicense> {
    let auth = auth::require_auth().await?;
    let config = expect_context::<Arc<dyn UserConfigStorage>>();
    config
        .get_content_license(auth.user_id)
        .await
        .map_err(InternalError::storage)
}

/// Sets the authenticated User's publication-wide Content License.
#[macros::server(skip_all)]
pub async fn set_content_license(license: ContentLicense) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let config = expect_context::<Arc<dyn UserConfigStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    let now = UtcInstant::now();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                storage::update_content_license_with_feed_events(
                    transaction,
                    users.as_ref(),
                    config.as_ref(),
                    posts.as_ref(),
                    feed_events.as_ref(),
                    storage::ContentLicenseUpdate {
                        user_id: auth.user_id,
                        license,
                    },
                    now,
                )
                .await
                .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Sets the authenticated user's default post format preference.
#[macros::server]
pub async fn set_default_post_format(format: PostFormat) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let config = expect_context::<Arc<dyn UserConfigStorage>>();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                storage::set_default_post_format(config.as_ref(), transaction, auth.user_id, format)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

#[cfg(test)]
mod tests {
    use super::{SetContentLicense, SetDefaultAudience, SetDefaultPostFormat};
    use common::{
        content_license::ContentLicense, render::PostFormat, visibility::DefaultAudience,
    };

    #[test]
    fn profile_setting_wires_reject_unknown_tokens() {
        let format: SetDefaultPostFormat = serde_qs::from_str("format=markdown").unwrap();
        assert_eq!(format.format, PostFormat::Markdown);
        assert!(serde_qs::from_str::<SetDefaultPostFormat>("format=bogus").is_err());

        let license: SetContentLicense = serde_qs::from_str("license=CC-BY-4.0").unwrap();
        assert_eq!(license.license, ContentLicense::CcBy4_0);
        assert!(serde_qs::from_str::<SetContentLicense>("license=MIT").is_err());
    }

    #[test]
    fn user_default_audience_wire_distinguishes_inherit_from_explicit() {
        let explicit: SetDefaultAudience = serde_qs::from_str("audience=public").unwrap();
        assert_eq!(explicit.audience, Some(DefaultAudience::Public));

        let inherited: SetDefaultAudience = serde_qs::from_str("").unwrap();
        assert_eq!(inherited.audience, None);
        assert!(serde_qs::from_str::<SetDefaultAudience>("audience=friends").is_err());
    }
}

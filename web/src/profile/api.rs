//! Profile wire DTOs and authenticated `#[server]` endpoints.
//!
//! The profile owns author-scoped settings because the authenticated User is the
//! publication. Site-wide settings are colocated only because `/profile` is their
//! operator UI; both server boundaries retain their distinct authorization rules.
//! Dual-compiled (host + wasm); the vertical's grouped server imports live here.

use crate::error::WebResult;
use common::{
    MutationOutcome, bio::Bio, display_name::DisplayName, email::Email, render::PostFormat,
    theme::Theme, username::Username,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth,
    crate::error::{InternalError, from_write_scope_error},
    leptos::prelude::*,
    std::sync::Arc,
    storage::{ProfileUpdate, SiteConfigStorage, UserConfigStorage, UserStorage, WriteScope},
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
    let write_scope = expect_context::<WriteScope>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                users
                    .update_profile(
                        transaction,
                        auth.user_id,
                        &ProfileUpdate {
                            display_name: display_name.as_ref(),
                            bio: bio.as_ref(),
                        },
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

/// Retrieves the caller's optional public-pages theme override.
#[macros::server]
pub async fn get_your_pages_theme() -> WebResult<Option<Theme>> {
    let auth = auth::require_auth().await?;
    let config = expect_context::<Arc<dyn UserConfigStorage>>();
    Ok(storage::get_theme_override(config.as_ref(), auth.user_id).await?)
}

/// Persists a public-pages theme override for the authenticated author.
#[macros::server]
pub async fn set_your_pages_theme(theme: Theme) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let config = expect_context::<Arc<dyn UserConfigStorage>>();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                storage::set_theme_override(config.as_ref(), transaction, auth.user_id, theme)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Deletes the authenticated author's override, restoring site-theme inheritance.
#[macros::server]
pub async fn reset_your_pages_theme() -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let config = expect_context::<Arc<dyn UserConfigStorage>>();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                storage::delete_theme_override(config.as_ref(), transaction, auth.user_id)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Retrieves the operator-owned default public theme.
#[macros::server]
pub async fn get_site_theme() -> WebResult<Theme> {
    auth::require_operator().await?;
    let config = expect_context::<Arc<dyn SiteConfigStorage>>();
    Ok(config.get_theme().await?)
}

/// Persists the operator-owned default public theme.
#[macros::server]
pub async fn set_site_theme(theme: Theme) -> WebResult<MutationOutcome<()>> {
    auth::require_operator().await?;
    let write_scope = expect_context::<WriteScope>();
    let config = expect_context::<Arc<dyn SiteConfigStorage>>();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                config
                    .set_theme(transaction, theme)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

#[cfg(test)]
mod tests {
    use super::{SetDefaultPostFormat, SetSiteTheme, SetYourPagesTheme};
    use common::{render::PostFormat, theme::Theme};

    #[test]
    fn set_default_post_format_wire_rejects_unknown_token() {
        let ok: SetDefaultPostFormat = serde_qs::from_str("format=markdown").unwrap();
        assert_eq!(ok.format, PostFormat::Markdown);
        assert!(serde_qs::from_str::<SetDefaultPostFormat>("format=bogus").is_err());
    }

    #[test]
    fn theme_mutation_wires_reject_unknown_tokens() {
        let author: SetYourPagesTheme = serde_qs::from_str("theme=terminal").unwrap();
        let site: SetSiteTheme = serde_qs::from_str("theme=reader").unwrap();

        assert_eq!(author.theme, Theme::Terminal);
        assert_eq!(site.theme, Theme::Reader);
        assert!(serde_qs::from_str::<SetYourPagesTheme>("theme=bogus").is_err());
        assert!(serde_qs::from_str::<SetSiteTheme>("theme=bogus").is_err());
    }
}

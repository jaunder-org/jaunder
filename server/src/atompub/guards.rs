use common::tagged_url::BaseUrl;
use common::username::Username;
use storage::SiteConfigStorage;
use web::auth::AuthUser;

use super::HandlerError;

/// Authorizes that `auth_user` may act on resources scoped to `username`.
///
/// `AtomPub` collection handlers are addressed by `{username}`; a user may only
/// act on their own resources, so a mismatch yields `403 Forbidden`. The member
/// handlers fold the same check into `owned_post`.
pub(crate) fn require_user_match(
    auth_user: &AuthUser,
    username: &Username,
) -> Result<(), HandlerError> {
    if auth_user.username == *username {
        Ok(())
    } else {
        Err(HandlerError::Forbidden)
    }
}

/// Returns the site's public base URL (an absolute `http(s)` origin), or `None`
/// when unconfigured (callers then emit root-relative URLs via
/// [`common::tagged_url::compose`]).
///
/// # Errors
///
/// Returns the identity-store failure unchanged.
pub(crate) async fn base_url(
    site_config: &dyn SiteConfigStorage,
) -> Result<Option<BaseUrl>, sqlx::Error> {
    site_config
        .get_identity()
        .await
        .map(|identity| identity.base_url)
}

/// [`base_url`] as a **required** precondition (#560): maps only the unset case
/// to [`HandlerError::BaseUrlRequired`]. Identity-store failures remain typed in
/// [`HandlerError::Internal`].
///
/// # Errors
///
/// Returns `BaseUrlRequired` for `Ok(None)` and `Internal` for a storage error.
#[doc(hidden)]
pub async fn required_base_url(
    site_config: &dyn SiteConfigStorage,
) -> Result<BaseUrl, HandlerError> {
    base_url(site_config)
        .await
        .map_err(HandlerError::from)?
        .ok_or(HandlerError::BaseUrlRequired)
}

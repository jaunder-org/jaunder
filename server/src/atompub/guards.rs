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
pub(crate) async fn base_url(site_config: &dyn SiteConfigStorage) -> Option<BaseUrl> {
    site_config
        .get_identity()
        .await
        .ok()
        .and_then(|identity| identity.base_url)
}

/// [`base_url`] as a **required** precondition (#560): maps the unset case to
/// [`HandlerError::BaseUrlRequired`] (logged as a `500` at the response boundary), so a
/// composed-URL handler narrows `SiteIdentity.base_url` to a [`BaseUrl`] in one `?`
/// and every downstream `compose` is infallible.
pub(crate) async fn required_base_url(
    site_config: &dyn SiteConfigStorage,
) -> Result<BaseUrl, HandlerError> {
    base_url(site_config)
        .await
        .ok_or(HandlerError::BaseUrlRequired)
}

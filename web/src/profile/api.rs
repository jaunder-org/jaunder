//! Profile wire DTOs + `#[server]` endpoints (ADR-0070, amended #530): the
//! `Data` payload and the `get` / `update` /
//! `get_default_post_format` / `set_default_post_format` server fns. Dual-compiled
//! (host + wasm); the vertical's one grouped `#[cfg(feature = "server")]` use-block
//! lives here. Re-exported from `mod.rs` so `crate::profile::…` paths stay stable.

use crate::error::WebResult;
use common::{
    MutationOutcome, bio::Bio, display_name::DisplayName, email::Email, render::PostFormat,
    username::Username,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth,
    crate::error::{InternalError, from_write_scope_error},
    leptos::prelude::*,
    std::sync::Arc,
    storage::{ProfileUpdate, UserConfigStorage, UserStorage, WriteScope},
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

#[cfg(test)]
mod tests {
    use super::SetDefaultPostFormat;
    use common::render::PostFormat;

    #[test]
    fn set_default_post_format_wire_rejects_unknown_token() {
        // The typed `SetDefaultPostFormat` dispatch encodes `format=<token>` over
        // server_fn's default Url codec (serde_qs); this test pins the endpoint's
        // decode contract independent of the client widget. A valid token decodes; a
        // bogus one is rejected at the wire boundary once the arg is a typed PostFormat.
        let ok: SetDefaultPostFormat = serde_qs::from_str("format=markdown").unwrap();
        assert_eq!(ok.format, PostFormat::Markdown);
        assert!(serde_qs::from_str::<SetDefaultPostFormat>("format=bogus").is_err());
    }
}

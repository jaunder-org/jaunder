use crate::error::WebResult;
// `Username` is ungated: it types the `#[server]` wire args below, so the generated
// arg structs reference it on both the client and server builds.
use common::username::Username;

#[cfg(feature = "server")]
use super::server::resolve_author;
#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{SubscriptionStorage, UserStorage},
};

/// Subscribes the authenticated local user to `author_username` on the
/// `local` channel.
///
/// Requires an authenticated local account (Layer A). Rejects a self-subscribe
/// and an unknown author. Idempotent: subscribing twice is a no-op.
#[macros::server]
pub async fn subscribe(author_username: Username) -> WebResult<()> {
    let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let auth = require_auth().await?;
    let author_id = resolve_author(users.as_ref(), &author_username, auth.user_id).await?;
    let channel_id = subscriptions.local_channel_id().await?;
    subscriptions
        .subscribe(author_id, channel_id, &i64::from(auth.user_id).to_string())
        .await?;
    Ok(())
}

/// Unsubscribes the authenticated local user from `author_username`.
///
/// Mirror of [`subscribe`]. A no-op if no subscription exists.
#[macros::server]
pub async fn unsubscribe(author_username: Username) -> WebResult<()> {
    let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let auth = require_auth().await?;
    let author_id = resolve_author(users.as_ref(), &author_username, auth.user_id).await?;
    let channel_id = subscriptions.local_channel_id().await?;
    subscriptions
        .unsubscribe(author_id, channel_id, &i64::from(auth.user_id).to_string())
        .await?;
    Ok(())
}

/// Reports whether the authenticated local user is subscribed to
/// `author_username` (drives the profile button state).
///
/// Returns `false` for an anonymous viewer or when viewing one's own profile
/// (self-subscription is impossible), so the caller can hide the control.
#[macros::server]
pub async fn is_subscribed(author_username: Username) -> WebResult<bool> {
    let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let auth = require_auth().await?;
    // `resolve_author` rejects a self-target; treat that as "not subscribed"
    // so the profile can hide the button rather than surfacing an error.
    let Ok(author_id) = resolve_author(users.as_ref(), &author_username, auth.user_id).await else {
        return Ok(false);
    };
    let channel_id = subscriptions.local_channel_id().await?;
    let viewer = common::visibility::ViewerIdentity::local(auth.user_id, channel_id);
    let subscribed = subscriptions.is_subscriber(author_id, &viewer).await?;
    Ok(subscribed)
}

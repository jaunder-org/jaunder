//! Subscribe / unsubscribe `#[server]` functions for the `local` channel.
//!
//! These power the Subscribe / Unsubscribe button on a user's profile
//! (timeline) page. Layer A only supports *local* subscriptions: the viewer
//! must be a logged-in local account, and the subscription is recorded on the
//! seeded `local` channel keyed by the viewer's `user_id` (rendered as the
//! `subscriber_ref`). Creation routes through the store's admission seam
//! (`OpenSubscriptionPolicy` → `active` in Layer A; an approval gate later).
//!
//! Self-subscription is rejected — an author cannot subscribe to themselves.

use crate::error::WebResult;
use leptos::prelude::*;

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    common::username::Username,
    std::sync::Arc,
    storage::{SubscriptionStorage, UserStorage},
};

/// Resolves an author `user_id` from a (trimmed) username, rejecting the
/// caller's own username (self-subscribe) and an unknown username.
///
/// Takes the `users` handle as a parameter rather than reading it from context,
/// keeping it a pure helper its callers wire up.
#[cfg(feature = "server")]
async fn resolve_author(
    users: &dyn UserStorage,
    author_username: &str,
    viewer_user_id: i64,
) -> Result<i64, InternalError> {
    let username = author_username
        .trim()
        .strip_prefix('~')
        .unwrap_or_else(|| author_username.trim())
        .parse::<Username>()
        .map_err(|e| InternalError::validation(e.to_string()))?;
    let author = users
        .get_user_by_username(&username)
        .await
        .map_err(InternalError::storage)?
        .ok_or_else(|| InternalError::not_found("user"))?;
    if author.user_id == viewer_user_id {
        return Err(InternalError::validation("cannot subscribe to yourself"));
    }
    Ok(author.user_id)
}

/// Subscribes the authenticated local user to `author_username` on the
/// `local` channel.
///
/// Requires an authenticated local account (Layer A). Rejects a self-subscribe
/// and an unknown author. Idempotent: subscribing twice is a no-op.
#[server(endpoint = "/subscribe_to")]
pub async fn subscribe_to(author_username: String) -> WebResult<()> {
    boundary!("subscribe_to", {
        let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let auth = require_auth().await?;
        let author_id = resolve_author(users.as_ref(), &author_username, auth.user_id).await?;
        let channel_id = subscriptions
            .local_channel_id()
            .await
            .map_err(InternalError::storage)?;
        subscriptions
            .subscribe(author_id, channel_id, &auth.user_id.to_string())
            .await
            .map_err(InternalError::storage)?;
        Ok(())
    })
}

/// Unsubscribes the authenticated local user from `author_username`.
///
/// Mirror of [`subscribe_to`]. A no-op if no subscription exists.
#[server(endpoint = "/unsubscribe_from")]
pub async fn unsubscribe_from(author_username: String) -> WebResult<()> {
    boundary!("unsubscribe_from", {
        let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let auth = require_auth().await?;
        let author_id = resolve_author(users.as_ref(), &author_username, auth.user_id).await?;
        let channel_id = subscriptions
            .local_channel_id()
            .await
            .map_err(InternalError::storage)?;
        subscriptions
            .unsubscribe(author_id, channel_id, &auth.user_id.to_string())
            .await
            .map_err(InternalError::storage)?;
        Ok(())
    })
}

/// Reports whether the authenticated local user is subscribed to
/// `author_username` (drives the profile button state).
///
/// Returns `false` for an anonymous viewer or when viewing one's own profile
/// (self-subscription is impossible), so the caller can hide the control.
#[server(endpoint = "/is_subscribed_to")]
pub async fn is_subscribed_to(author_username: String) -> WebResult<bool> {
    boundary!("is_subscribed_to", {
        let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let auth = require_auth().await?;
        // `resolve_author` rejects a self-target; treat that as "not subscribed"
        // so the profile can hide the button rather than surfacing an error.
        let Ok(author_id) = resolve_author(users.as_ref(), &author_username, auth.user_id).await
        else {
            return Ok(false); // cov:ignore
        };
        let channel_id = subscriptions
            .local_channel_id()
            .await
            .map_err(InternalError::storage)?;
        let viewer = common::visibility::ViewerIdentity::local(auth.user_id, channel_id);
        let subscribed = subscriptions
            .is_subscriber(author_id, &viewer)
            .await
            .map_err(InternalError::storage)?;
        Ok(subscribed)
    })
}

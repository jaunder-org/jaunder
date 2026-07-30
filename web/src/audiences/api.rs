//! The `#[server]` endpoints for named-audience management and the wire DTOs they
//! exchange. See the module doc on [`super`] for the authorization model.

use crate::error::WebResult;
// `AudienceName` is the wire-arg type of `create` / `rename`, so the
// `#[server]`-generated arg structs reference it on both the client and server builds —
// keep it ungated.
use common::audience::AudienceName;
use common::ids::{AudienceId, SubscriptionId};
use reactive_stores::{Patch, Store};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    common::ids::UserId,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{AudienceStorage, SubscriptionStorage, UserStorage},
};

/// A named audience as shown in the management screen.
///
/// A `reactive_stores` keyed-store row (`Store`/`Patch`), so each field carries
/// `#[patch(|this, new| *this = new)]` — the derive's escape hatch, which lets the
/// fields keep their domain types instead of being flattened to `i64`/`String`.
/// Rationale and the rejected alternatives:
/// `docs/adr/0078-reactive-store-domain-newtype-fields.md`.
///
/// `audience_id`'s attribute is required to compile but is behaviorally inert: it is
/// the store key, so `patch_field_keyed` has already matched the two rows *by* it
/// before the closure is reached, and the value can never differ. Only `name`'s
/// attribute does real work — and it is the one the audiences e2e guards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
pub struct Summary {
    #[patch(|this, new| *this = new)]
    pub audience_id: AudienceId,
    #[patch(|this, new| *this = new)]
    pub name: AudienceName,
}

/// One of the author's active subscribers, for the assignment checklist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriberSummary {
    pub subscription_id: SubscriptionId,
    /// The local subscriber's username (resolved from `subscriber_ref`), or the
    /// raw reference when it could not be resolved to a local user.
    pub label: String,
}

/// Creates a named audience owned by the authenticated author.
#[macros::server(skip_all)]
pub async fn create(name: AudienceName) -> WebResult<AudienceId> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = require_auth().await?;
    // `name` arrives already validated (typed wire arg, client-pre-validated via the
    // direct-bound `AudienceName` field, per ADR-0065): its serde bridge routes
    // through `AudienceName::from_str`, so the empty/whitespace rule ran on decode.
    let id = audiences.create_audience(auth.user_id, &name).await?;
    Ok(id)
}

/// Renames an audience the authenticated author owns.
#[macros::server(skip(name))]
pub async fn rename(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = require_auth().await?;
    // `name` arrives already validated (see `create`).
    audiences
        .rename_audience(auth.user_id, audience_id, &name)
        .await?;
    Ok(())
}

/// Deletes an audience the authenticated author owns (and its memberships).
#[macros::server]
pub async fn delete(audience_id: AudienceId) -> WebResult<()> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = require_auth().await?;
    audiences.delete_audience(auth.user_id, audience_id).await?;
    Ok(())
}

/// Lists the authenticated author's named audiences.
#[macros::server]
pub async fn list_mine() -> WebResult<Vec<Summary>> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = require_auth().await?;
    let rows = audiences.list_audiences(auth.user_id).await?;
    Ok(rows
        .into_iter()
        .map(|a| Summary {
            audience_id: a.audience_id,
            name: a.name,
        })
        .collect())
}

/// Lists the authenticated author's active subscribers (for the assignment
/// checklist). Resolves each local `subscriber_ref` to a username for display.
#[macros::server]
pub async fn list_my_subscribers() -> WebResult<Vec<SubscriberSummary>> {
    let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let auth = require_auth().await?;
    let rows = subscriptions.list_subscribers(auth.user_id).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        // `subscriber_ref` is the local user id (as a string) for the local
        // channel. Resolve it to a username for display; fall back to the
        // raw reference if it cannot be resolved.
        let label = match row.subscriber_ref.parse::<i64>() {
            Ok(uid) => users
                .get_user(UserId::from(uid))
                .await
                .ok()
                .flatten()
                .map_or_else(|| row.subscriber_ref.clone(), |u| u.username.to_string()),
            Err(_) => row.subscriber_ref.clone(),
        };
        out.push(SubscriberSummary {
            subscription_id: row.subscription_id,
            label,
        });
    }
    Ok(out)
}

/// Adds a subscription to an audience, both owned by the authenticated author.
///
/// `add_member` is author-scoped in the store (it writes `author_user_id` so
/// the composite FKs reject a cross-author pairing), so passing the session's
/// `user_id` is the authorization.
#[macros::server]
pub async fn add_subscriber(
    audience_id: AudienceId,
    subscription_id: SubscriptionId,
) -> WebResult<()> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = require_auth().await?;
    audiences
        .add_member(auth.user_id, audience_id, subscription_id)
        .await?;
    Ok(())
}

/// Removes a subscription from an audience the authenticated author owns.
/// `remove_member` is author-scoped, so a cross-author `audience_id` is a no-op.
#[macros::server]
pub async fn remove_subscriber(
    audience_id: AudienceId,
    subscription_id: SubscriptionId,
) -> WebResult<()> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = require_auth().await?;
    audiences
        .remove_member(auth.user_id, audience_id, subscription_id)
        .await?;
    Ok(())
}

/// Lists the `subscription_id`s assigned to an audience the author owns.
/// `list_members` is author-scoped, so a cross-author `audience_id` lists empty.
#[macros::server]
pub async fn list_members(audience_id: AudienceId) -> WebResult<Vec<SubscriptionId>> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = require_auth().await?;
    let members = audiences.list_members(auth.user_id, audience_id).await?;
    Ok(members)
}

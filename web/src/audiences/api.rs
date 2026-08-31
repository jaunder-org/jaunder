//! The `#[server]` endpoints for named-audience management and the wire DTOs they
//! exchange. See the module doc on [`super`] for the authorization model.

use super::model::{SubscriberSummary, Summary};
use crate::error::WebResult;
use common::ids::{AudienceId, SubscriptionId};
use common::{MutationOutcome, audience::AudienceName};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    super::model,
    crate::auth,
    crate::error::{InternalError, from_write_scope_error},
    leptos::prelude::*,
    std::sync::Arc,
    storage::{AudienceStorage, SubscriptionStorage, WriteScope},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameAudienceRequest {
    pub audience_id: AudienceId,
    pub name: AudienceName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudienceMembershipRequest {
    pub audience_id: AudienceId,
    pub subscription_id: SubscriptionId,
}

/// Creates a named audience owned by the authenticated author.
#[macros::server(skip_all)]
pub async fn create(name: AudienceName) -> WebResult<MutationOutcome<AudienceId>> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    // `name` arrives already validated (typed wire arg, client-pre-validated via the
    // direct-bound `AudienceName` field, per ADR-0065): its serde bridge routes
    // through `AudienceName::from_str`, so the empty/whitespace rule ran on decode.
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .create_audience(transaction, auth.user_id, &name)
                    .await
                    .map_err(InternalError::from)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Renames an audience the authenticated author owns.
#[macros::server(skip_all)]
pub async fn rename(request: RenameAudienceRequest) -> WebResult<MutationOutcome<()>> {
    let RenameAudienceRequest { audience_id, name } = request;
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    // `name` arrives already validated (see `create`).
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .rename_audience(transaction, auth.user_id, audience_id, &name)
                    .await
                    .map_err(InternalError::from)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Deletes an audience the authenticated author owns (and its memberships).
#[macros::server]
pub async fn delete(audience_id: AudienceId) -> WebResult<MutationOutcome<()>> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .delete_audience(transaction, auth.user_id, audience_id)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Lists the authenticated author's named audiences.
#[macros::server]
pub async fn list_mine() -> WebResult<Vec<Summary>> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = auth::require_auth().await?;
    model::list_audiences(auth.user_id, audiences.as_ref()).await
}

/// Lists the authenticated author's active subscribers (for the assignment
/// checklist). Resolves each local `subscriber_ref` to a username for display.
#[macros::server]
pub async fn list_my_subscribers() -> WebResult<Vec<SubscriberSummary>> {
    let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
    let auth = auth::require_auth().await?;
    model::list_subscribers(auth.user_id, subscriptions.as_ref()).await
}

/// Adds a subscription to an audience, both owned by the authenticated author.
///
/// `add_member` is author-scoped in the store (it writes `author_user_id` so
/// the composite FKs reject a cross-author pairing), so passing the session's
/// `user_id` is the authorization.
#[macros::server(skip_all)]
pub async fn add_subscriber(request: AudienceMembershipRequest) -> WebResult<MutationOutcome<()>> {
    let AudienceMembershipRequest {
        audience_id,
        subscription_id,
    } = request;
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .add_member(transaction, auth.user_id, audience_id, subscription_id)
                    .await
                    .map_err(InternalError::from)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Removes a subscription from an audience the authenticated author owns.
/// `remove_member` is author-scoped, so a cross-author `audience_id` is a no-op.
#[macros::server(skip_all)]
pub async fn remove_subscriber(
    request: AudienceMembershipRequest,
) -> WebResult<MutationOutcome<()>> {
    let AudienceMembershipRequest {
        audience_id,
        subscription_id,
    } = request;
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    write_scope
        .run(move |transaction| {
            Box::pin(async move {
                audiences
                    .remove_member(transaction, auth.user_id, audience_id, subscription_id)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Lists the `subscription_id`s assigned to an audience the author owns.
/// `list_members` is author-scoped, so a cross-author `audience_id` lists empty.
#[macros::server]
pub async fn list_members(audience_id: AudienceId) -> WebResult<Vec<SubscriptionId>> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    let auth = auth::require_auth().await?;
    let members = audiences.list_members(auth.user_id, audience_id).await?;
    Ok(members)
}

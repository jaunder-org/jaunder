//! The `#[server]` endpoints for named-audience management and the wire DTOs they
//! exchange. See the module doc on [`super`] for the authorization model.

use crate::error::WebResult;
// `AudienceName` is the wire-arg type of `create_audience` / `rename_audience`, so the
// `#[server]`-generated arg structs reference it on both the client and server builds —
// keep it ungated.
use common::audience::AudienceName;
use common::ids::{AudienceId, SubscriptionId};
use leptos::prelude::*;
use reactive_stores::{Patch, Store};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    common::ids::UserId,
    std::sync::Arc,
    storage::{AudienceStorage, SubscriptionStorage, UserStorage},
};

/// A named audience as shown in the management screen.
///
/// `audience_id` stays a bare `i64` here (not `AudienceId`): this is a
/// `reactive_stores` keyed-store row (`Store`/`Patch`), and `Patch` requires the
/// field to be `PatchField` — a foreign trait implemented only for primitives, with
/// no blanket impl. Typing it would force `impl PatchField for AudienceId`; that impl
/// is coherent only in `common` (where `AudienceId` is defined), but that would drag a
/// leptos-client dependency (`reactive_stores`) into the backend-agnostic crate
/// (ADR-0055/0058) — and in `web` it is an outright orphan violation. So this one
/// reactive surface holds the primitive and
/// converts at its edges (built from `AudienceRecord`; wrapped into `AudienceId`
/// where it flows into the typed components/server fns) — the ADR-0063
/// external-non-owned-type carve-out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
pub struct AudienceSummary {
    pub audience_id: i64,
    pub name: String,
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
#[server(endpoint = "/create_audience")]
pub async fn create_audience(name: AudienceName) -> WebResult<AudienceId> {
    boundary!("create_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        // `name` arrives already validated (typed wire arg, client-pre-validated via the
        // direct-bound `AudienceName` field, per ADR-0065): its serde bridge routes
        // through `AudienceName::from_str`, so the empty/whitespace rule ran on decode.
        let id = audiences.create_audience(auth.user_id, &name).await?;
        Ok(id)
    })
}

/// Renames an audience the authenticated author owns.
#[server(endpoint = "/rename_audience")]
pub async fn rename_audience(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
    boundary!("rename_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        // `name` arrives already validated (see `create_audience`).
        audiences
            .rename_audience(auth.user_id, audience_id, &name)
            .await?;
        Ok(())
    })
}

/// Deletes an audience the authenticated author owns (and its memberships).
#[server(endpoint = "/delete_audience")]
pub async fn delete_audience(audience_id: AudienceId) -> WebResult<()> {
    boundary!("delete_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        audiences.delete_audience(auth.user_id, audience_id).await?;
        Ok(())
    })
}

/// Lists the authenticated author's named audiences.
#[server(endpoint = "/list_my_audiences")]
pub async fn list_my_audiences() -> WebResult<Vec<AudienceSummary>> {
    boundary!("list_my_audiences", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        let rows = audiences.list_audiences(auth.user_id).await?;
        Ok(rows
            .into_iter()
            .map(|a| AudienceSummary {
                audience_id: i64::from(a.audience_id),
                name: a.name,
            })
            .collect())
    })
}

/// Lists the authenticated author's active subscribers (for the assignment
/// checklist). Resolves each local `subscriber_ref` to a username for display.
#[server(endpoint = "/list_my_subscribers")]
pub async fn list_my_subscribers() -> WebResult<Vec<SubscriberSummary>> {
    boundary!("list_my_subscribers", {
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
    })
}

/// Adds a subscription to an audience, both owned by the authenticated author.
///
/// `add_member` is author-scoped in the store (it writes `author_user_id` so
/// the composite FKs reject a cross-author pairing), so passing the session's
/// `user_id` is the authorization.
#[server(endpoint = "/add_subscriber_to_audience")]
pub async fn add_subscriber_to_audience(
    audience_id: AudienceId,
    subscription_id: SubscriptionId,
) -> WebResult<()> {
    boundary!("add_subscriber_to_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        audiences
            .add_member(auth.user_id, audience_id, subscription_id)
            .await?;
        Ok(())
    })
}

/// Removes a subscription from an audience the authenticated author owns.
/// `remove_member` is author-scoped, so a cross-author `audience_id` is a no-op.
#[server(endpoint = "/remove_subscriber_from_audience")]
pub async fn remove_subscriber_from_audience(
    audience_id: AudienceId,
    subscription_id: SubscriptionId,
) -> WebResult<()> {
    boundary!("remove_subscriber_from_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        audiences
            .remove_member(auth.user_id, audience_id, subscription_id)
            .await?;
        Ok(())
    })
}

/// Lists the `subscription_id`s assigned to an audience the author owns.
/// `list_members` is author-scoped, so a cross-author `audience_id` lists empty.
#[server(endpoint = "/list_audience_members")]
pub async fn list_audience_members(audience_id: AudienceId) -> WebResult<Vec<SubscriptionId>> {
    boundary!("list_audience_members", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        let members = audiences.list_members(auth.user_id, audience_id).await?;
        Ok(members)
    })
}

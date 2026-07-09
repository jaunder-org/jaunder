//! Named-audience management `#[server]` functions for the account area.
//!
//! These let an author curate named groups of their own active subscribers and
//! assign/unassign subscribers to those groups. They back the Audiences screen
//! under the account/settings nav and feed the post-editor audience picker.
//!
//! ## Authorization
//!
//! Every function derives `author_user_id` from the authenticated session
//! ([`require_auth`]) — **never** from a client parameter. The store's mutating
//! methods are author-scoped (`create_audience`, `rename_audience`,
//! `delete_audience`, `add_member`), so passing the session's `user_id` is
//! sufficient there. But [`AudienceStorage::remove_member`] and
//! [`AudienceStorage::list_members`] are **not** author-scoped in the store — a
//! client-supplied `audience_id` could otherwise reach another author's
//! audience. So before calling them, these functions verify the target
//! `audience_id` belongs to the authed author (it must appear in
//! `list_audiences(author)`), rejecting otherwise.

use crate::error::WebResult;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    std::sync::Arc,
    storage::{AudienceError, AudienceStorage, SubscriptionStorage},
};

/// A named audience as shown in the management screen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudienceSummary {
    pub audience_id: i64,
    pub name: String,
}

/// One of the author's active subscribers, for the assignment checklist.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriberSummary {
    pub subscription_id: i64,
    /// The local subscriber's username (resolved from `subscriber_ref`), or the
    /// raw reference when it could not be resolved to a local user.
    pub label: String,
}

/// Maps an [`AudienceError`] to a user-facing [`InternalError`]: duplicate
/// names and missing audiences are client-correctable; everything else is a
/// masked storage failure.
#[cfg(feature = "server")]
fn map_audience_error(err: AudienceError) -> InternalError {
    match err {
        AudienceError::DuplicateName => {
            InternalError::conflict("an audience with that name already exists")
        }
        AudienceError::NotFound => InternalError::not_found("audience"),
        AudienceError::Storage(e) => InternalError::storage(e),
    }
}

/// Confirms `audience_id` belongs to `author_user_id` by checking it appears in
/// the author's own audience list. This is the ownership gate for the store
/// methods that are not author-scoped (`remove_member`, `list_members`).
#[cfg(feature = "server")]
async fn assert_owns_audience(
    audiences: &dyn AudienceStorage,
    author_user_id: i64,
    audience_id: i64,
) -> Result<(), InternalError> {
    let owned = audiences
        .list_audiences(author_user_id)
        .await
        .map_err(InternalError::storage)?;
    if owned.iter().any(|a| a.audience_id == audience_id) {
        Ok(())
    } else {
        Err(InternalError::not_found("audience"))
    }
}

/// Creates a named audience owned by the authenticated author.
#[server(endpoint = "/create_audience")]
pub async fn create_audience(name: String) -> WebResult<i64> {
    boundary!("create_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        let name = name.trim();
        if name.is_empty() {
            return Err(InternalError::validation("audience name must not be empty"));
        }
        let id = audiences
            .create_audience(auth.user_id, name)
            .await
            .map_err(map_audience_error)?;
        Ok(id)
    })
}

/// Renames an audience the authenticated author owns.
#[server(endpoint = "/rename_audience")]
pub async fn rename_audience(audience_id: i64, name: String) -> WebResult<()> {
    boundary!("rename_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        let name = name.trim();
        if name.is_empty() {
            return Err(InternalError::validation("audience name must not be empty"));
        }
        audiences
            .rename_audience(auth.user_id, audience_id, name)
            .await
            .map_err(map_audience_error)?;
        Ok(())
    })
}

/// Deletes an audience the authenticated author owns (and its memberships).
#[server(endpoint = "/delete_audience")]
pub async fn delete_audience(audience_id: i64) -> WebResult<()> {
    boundary!("delete_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        audiences
            .delete_audience(auth.user_id, audience_id)
            .await
            .map_err(InternalError::storage)?;
        Ok(())
    })
}

/// Lists the authenticated author's named audiences.
#[server(endpoint = "/list_my_audiences")]
pub async fn list_my_audiences() -> WebResult<Vec<AudienceSummary>> {
    boundary!("list_my_audiences", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        let rows = audiences
            .list_audiences(auth.user_id)
            .await
            .map_err(InternalError::storage)?;
        Ok(rows
            .into_iter()
            .map(|a| AudienceSummary {
                audience_id: a.audience_id,
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
        use storage::UserStorage;
        let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let auth = require_auth().await?;
        let rows = subscriptions
            .list_subscribers(auth.user_id)
            .await
            .map_err(InternalError::storage)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            // `subscriber_ref` is the local user id (as a string) for the local
            // channel. Resolve it to a username for display; fall back to the
            // raw reference if it cannot be resolved.
            let label = match row.subscriber_ref.parse::<i64>() {
                Ok(uid) => users
                    .get_user(uid)
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
pub async fn add_subscriber_to_audience(audience_id: i64, subscription_id: i64) -> WebResult<()> {
    boundary!("add_subscriber_to_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        audiences
            .add_member(auth.user_id, audience_id, subscription_id)
            .await
            .map_err(map_audience_error)?;
        Ok(())
    })
}

/// Removes a subscription from an audience the authenticated author owns.
///
/// `remove_member` is **not** author-scoped in the store, so we verify
/// ownership of `audience_id` before calling it.
#[server(endpoint = "/remove_subscriber_from_audience")]
pub async fn remove_subscriber_from_audience(
    audience_id: i64,
    subscription_id: i64,
) -> WebResult<()> {
    boundary!("remove_subscriber_from_audience", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        assert_owns_audience(audiences.as_ref(), auth.user_id, audience_id).await?;
        audiences
            .remove_member(audience_id, subscription_id)
            .await
            .map_err(InternalError::storage)?;
        Ok(())
    })
}

/// Lists the `subscription_id`s assigned to an audience the author owns.
///
/// `list_members` is **not** author-scoped in the store, so we verify
/// ownership of `audience_id` before calling it.
#[server(endpoint = "/list_audience_members")]
pub async fn list_audience_members(audience_id: i64) -> WebResult<Vec<i64>> {
    boundary!("list_audience_members", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        assert_owns_audience(audiences.as_ref(), auth.user_id, audience_id).await?;
        let members = audiences
            .list_members(audience_id)
            .await
            .map_err(InternalError::storage)?;
        Ok(members)
    })
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{list_my_subscribers, map_audience_error};
    use crate::error::WebError;
    use crate::test_support::auth_parts;
    use common::visibility::SubscriptionStatus;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{
        AudienceError, MockSubscriptionStorage, MockUserStorage, SubscriptionRecord,
        SubscriptionStorage, UserStorage,
    };

    #[test]
    fn map_audience_error_not_found_maps_to_not_found() {
        let err = map_audience_error(AudienceError::NotFound);
        assert!(matches!(
            crate::error::project(err.kind(), err.public_message()),
            WebError::NotFound { .. }
        ));
    }

    #[test]
    fn map_audience_error_storage_maps_to_storage() {
        let err = map_audience_error(AudienceError::Storage(sqlx::Error::PoolClosed));
        assert!(matches!(
            crate::error::project(err.kind(), err.public_message()),
            WebError::Storage { .. }
        ));
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn list_my_subscribers_falls_back_to_raw_ref_when_non_numeric() {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(1, "alice"));
        let mut subs = MockSubscriptionStorage::new();
        subs.expect_list_subscribers().returning(|_author| {
            Ok(vec![SubscriptionRecord {
                subscription_id: 7,
                channel_id: 1,
                subscriber_ref: "not-a-number".to_string(),
                status: SubscriptionStatus::Active,
                created_at: chrono::Utc::now(),
            }])
        });
        provide_context(Arc::new(subs) as Arc<dyn SubscriptionStorage>);
        // A non-numeric `subscriber_ref` never parses to a user id, so `get_user`
        // is never called; the raw reference is used as the display label.
        provide_context(Arc::new(MockUserStorage::new()) as Arc<dyn UserStorage>);

        let result = list_my_subscribers().await.unwrap();
        drop(owner);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "not-a-number");
    }
}

//! Named-audience management for the account area: the `#[server]` functions and
//! the co-located reactive UI (`AudiencesPage` and its child components).
//!
//! These let an author curate named groups of their own active subscribers and
//! assign/unassign subscribers to those groups. They back the Audiences screen
//! under the account/settings nav and feed the post-editor audience picker.
//!
//! ## Authorization
//!
//! Every function derives `author_user_id` from the authenticated session
//! ([`require_auth`]) — **never** from a client parameter. Every store method is
//! author-scoped (it takes `author_user_id` and filters by it), so passing the
//! session's `user_id` is the whole authorization: a client-supplied
//! `audience_id` owned by another author matches nothing (an empty list, or a
//! no-op delete).

use crate::error::WebResult;
use crate::ui::Topbar;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    std::sync::Arc,
    storage::{AudienceStorage, SubscriptionStorage, UserStorage},
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
        let id = audiences.create_audience(auth.user_id, name).await?;
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
            .await?;
        Ok(())
    })
}

/// Deletes an audience the authenticated author owns (and its memberships).
#[server(endpoint = "/delete_audience")]
pub async fn delete_audience(audience_id: i64) -> WebResult<()> {
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
            .await?;
        Ok(())
    })
}

/// Removes a subscription from an audience the authenticated author owns.
/// `remove_member` is author-scoped, so a cross-author `audience_id` is a no-op.
#[server(endpoint = "/remove_subscriber_from_audience")]
pub async fn remove_subscriber_from_audience(
    audience_id: i64,
    subscription_id: i64,
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
pub async fn list_audience_members(audience_id: i64) -> WebResult<Vec<i64>> {
    boundary!("list_audience_members", {
        let audiences = expect_context::<Arc<dyn AudienceStorage>>();
        let auth = require_auth().await?;
        let members = audiences.list_members(auth.user_id, audience_id).await?;
        Ok(members)
    })
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::list_my_subscribers;
    use crate::test_support::auth_parts;
    use common::visibility::SubscriptionStatus;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{
        MockSubscriptionStorage, MockUserStorage, SubscriptionRecord, SubscriptionStorage,
        UserStorage,
    };

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

/// Shared re-fetch triggers, provided by `AudiencesPage` and bumped from
/// context by each mutation-owning form (so no `ServerAction` is hoisted or
/// drilled). Split so a membership toggle re-fetches only members and never
/// rebuilds — remounts — the whole audience list.
#[derive(Clone, Copy)]
struct Revalidate {
    /// Bumped by create/rename/delete; the audience-list resource re-fetches.
    /// (Each `MemberChecklist` owns a *local* trigger for its own members, so a
    /// membership toggle re-fetches only that audience — never the list.)
    list: RwSignal<u32>,
}

/// Account-area screen for managing named audiences: lists the author's
/// audiences with create / rename / delete, and per audience an assign/unassign
/// checklist over their active subscribers.
#[component]
pub fn AudiencesPage() -> impl IntoView {
    let revalidate = Revalidate {
        list: RwSignal::new(0),
    };
    provide_context(revalidate);

    let audiences_res =
        crate::server_resource(move || revalidate.list.get(), |_| list_my_audiences());
    // The subscriber roster is independent of audience/membership ops, so it is
    // fetched once (constant source).
    let subscribers_res = crate::server_resource(|| (), |()| list_my_subscribers());

    // Copy each resolved resource into a signal, updating only on `Some`, so a
    // re-fetch retains the last value instead of flashing "Loading…"
    // (web-style-guide §9, as `home.rs`). `None` only until the first resolve.
    let audiences = RwSignal::new(None::<Result<Vec<AudienceSummary>, String>>);
    let subscribers = RwSignal::new(Vec::<SubscriberSummary>::new());
    Effect::new(move |_| {
        if let Some(result) = audiences_res.get() {
            audiences.set(Some(result.map_err(|e| e.to_string())));
        }
    });
    Effect::new(move |_| {
        if let Some(Ok(list)) = subscribers_res.get() {
            subscribers.set(list);
        }
    });

    view! {
        <Topbar title="Audiences".to_string() sub="Named subscriber groups".to_string() />
        <div class="j-scroll">
            <div class="j-page">
                <CreateAudienceForm />

                <section class="j-card">
                    <div class="j-card-head">
                        <div>
                            <h2>"Your audiences"</h2>
                            <div class="j-sub">
                                "Rename, delete, or assign subscribers to each audience."
                            </div>
                        </div>
                    </div>
                    {move || {
                        let subscribers = subscribers.get();
                        match audiences.get() {
                            None => view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any(),
                            Some(Ok(list)) if list.is_empty() => {
                                view! { <p>"No audiences yet."</p> }.into_any()
                            }
                            Some(Ok(list)) => {
                                view! {
                                    <ul class="j-audience-list">
                                        {list
                                            .into_iter()
                                            .map(|audience| {
                                                view! {
                                                    <AudienceRow
                                                        audience=audience
                                                        subscribers=subscribers.clone()
                                                    />
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </ul>
                                }
                                    .into_any()
                            }
                            Some(Err(e)) => view! { <p class="error">{e}</p> }.into_any(),
                        }
                    }}
                </section>
            </div>
        </div>
    }
}

/// The "Create an audience" card: owns the create action and re-fetches the
/// audience list on a successful create by bumping [`Revalidate`].
#[component]
fn CreateAudienceForm() -> impl IntoView {
    let revalidate = expect_context::<Revalidate>();
    let create_action = ServerAction::<CreateAudience>::new();
    Effect::new(move |_| {
        if matches!(create_action.value().get(), Some(Ok(_))) {
            revalidate.list.update(|n| *n += 1);
        }
    });

    view! {
        <section class="j-card">
            <div class="j-card-head">
                <div>
                    <h2>"Create an audience"</h2>
                    <div class="j-sub">
                        "Group your subscribers so you can target posts to a named set."
                    </div>
                </div>
            </div>
            <ActionForm action=create_action>
                <input type="text" name="name" placeholder="Audience name" required />
                <button type="submit" class="j-btn is-primary">
                    "Create"
                </button>
            </ActionForm>
            {move || {
                create_action
                    .value()
                    .get()
                    .and_then(Result::err)
                    .map(|e| view! { <p class="error">{e.to_string()}</p> })
            }}
        </section>
    }
}

/// One audience: its name with rename/delete controls and a checklist of the
/// author's active subscribers (checked = member).
#[component]
fn AudienceRow(audience: AudienceSummary, subscribers: Vec<SubscriberSummary>) -> impl IntoView {
    let audience_id = audience.audience_id;
    let name = audience.name.clone();
    view! {
        <li class="j-audience-item">
            <h3 class="j-audience-name">{name}</h3>
            <AudienceHeader audience=audience />
            <MemberChecklist audience_id=audience_id subscribers=subscribers />
        </li>
    }
}

/// The `j-audience-head` controls: rename and delete forms for one audience.
/// Owns both actions and bumps [`Revalidate`] on either success.
#[component]
fn AudienceHeader(audience: AudienceSummary) -> impl IntoView {
    let revalidate = expect_context::<Revalidate>();
    let rename_action = ServerAction::<RenameAudience>::new();
    let delete_action = ServerAction::<DeleteAudience>::new();
    Effect::new(move |_| {
        if matches!(rename_action.value().get(), Some(Ok(()))) {
            revalidate.list.update(|n| *n += 1);
        }
    });
    Effect::new(move |_| {
        if matches!(delete_action.value().get(), Some(Ok(()))) {
            revalidate.list.update(|n| *n += 1);
        }
    });

    let audience_id = audience.audience_id;
    let name = audience.name;
    view! {
        <div class="j-audience-head">
            <ActionForm action=rename_action>
                <input type="hidden" name="audience_id" value=audience_id />
                <input type="text" name="name" value=name />
                <button type="submit" class="j-btn">
                    "Rename"
                </button>
            </ActionForm>
            <ActionForm action=delete_action>
                <input type="hidden" name="audience_id" value=audience_id />
                <button type="submit" class="j-btn is-danger">
                    "Delete"
                </button>
            </ActionForm>
        </div>
    }
}

/// Per-subscriber add/remove checklist for one audience. Owns the add/remove
/// actions, fetches the current members, and bumps [`Revalidate`] on either
/// success.
#[component]
fn MemberChecklist(audience_id: i64, subscribers: Vec<SubscriberSummary>) -> impl IntoView {
    // A trigger local to this checklist: an add/remove here re-fetches only this
    // audience's members, not every audience's (and never the list).
    let members_revalidate = RwSignal::new(0u32);
    let add_action = ServerAction::<AddSubscriberToAudience>::new();
    let remove_action = ServerAction::<RemoveSubscriberFromAudience>::new();
    Effect::new(move |_| {
        if matches!(add_action.value().get(), Some(Ok(()))) {
            members_revalidate.update(|n| *n += 1);
        }
    });
    Effect::new(move |_| {
        if matches!(remove_action.value().get(), Some(Ok(()))) {
            members_revalidate.update(|n| *n += 1);
        }
    });

    let members_res = crate::server_resource(
        move || members_revalidate.get(),
        move |_| list_audience_members(audience_id),
    );
    // Sticky: retain the last member list across a re-fetch so a toggle does
    // not flash "Loading members…" (as `AudiencesPage`). `None` until first resolve.
    let member_ids = RwSignal::new(None::<Vec<i64>>);
    Effect::new(move |_| {
        if let Some(result) = members_res.get() {
            member_ids.set(Some(result.unwrap_or_default()));
        }
    });

    view! {
        {move || {
            let subscribers = subscribers.clone();
            match member_ids.get() {
                None => view! { <p class="j-loading">"Loading members\u{2026}"</p> }.into_any(),
                Some(member_ids) => {
                    if subscribers.is_empty() {
                        return view! { <p class="j-sub">"No active subscribers yet."</p> }
                            .into_any();
                    }
                    view! {
                        <ul class="j-audience-members">
                            {subscribers
                                .into_iter()
                                .map(|sub| {
                                    let is_member = member_ids.contains(&sub.subscription_id);
                                    let subscription_id = sub.subscription_id;
                                    let label = sub.label.clone();
                                    if is_member {
                                        view! {
                                            <li>
                                                <ActionForm action=remove_action>
                                                    <input type="hidden" name="audience_id" value=audience_id />
                                                    <input
                                                        type="hidden"
                                                        name="subscription_id"
                                                        value=subscription_id
                                                    />
                                                    <span class="j-audience-member is-member">{label}</span>
                                                    <button type="submit" class="j-btn">
                                                        "Remove"
                                                    </button>
                                                </ActionForm>
                                            </li>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <li>
                                                <ActionForm action=add_action>
                                                    <input type="hidden" name="audience_id" value=audience_id />
                                                    <input
                                                        type="hidden"
                                                        name="subscription_id"
                                                        value=subscription_id
                                                    />
                                                    <span class="j-audience-member">{label}</span>
                                                    <button type="submit" class="j-btn">
                                                        "Add"
                                                    </button>
                                                </ActionForm>
                                            </li>
                                        }
                                            .into_any()
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </ul>
                    }
                        .into_any()
                }
            }
        }}
    }
}

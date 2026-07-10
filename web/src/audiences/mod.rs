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
use crate::reactive::{invalidator_scope, Invalidator, ListState};
use crate::ui::Topbar;
use leptos::prelude::*;
use reactive_stores::{Field, Patch, Store};
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    std::sync::Arc,
    storage::{AudienceStorage, SubscriptionStorage, UserStorage},
};

/// A named audience as shown in the management screen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
pub struct AudienceSummary {
    pub audience_id: i64,
    pub name: String,
}

/// The reactive store backing the audience list: a keyed collection so a refetch
/// `patch`es row-identically (only changed rows' subfields notify), never remounting
/// unchanged rows. Distinct from `AudienceList` (#359's invalidator scope).
#[derive(Default, Store, Patch)]
struct AudienceListData {
    #[store(key: i64 = |a| a.audience_id)]
    audiences: Vec<AudienceSummary>,
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

invalidator_scope! {
    /// The audience-list refetch scope: `AudiencesPage` provides it; the create / rename /
    /// delete forms `notify` it (so no `ServerAction` is hoisted or drilled).
    struct AudienceList
}

/// Account-area screen for managing named audiences: lists the author's
/// audiences with create / rename / delete, and per audience an assign/unassign
/// checklist over their active subscribers.
#[component]
pub fn AudiencesPage() -> impl IntoView {
    // The audience list: a keyed reactive store, refetched via the `AudienceList` invalidator
    // and `patch`ed in place on success (`Invalidator::patched` owns the plumbing) — so
    // unchanged rows keep their DOM (and their `MemberChecklist`'s loaded members) and a rename
    // updates just that row's name. `state` drives the sibling loading/empty/error node.
    let list = AudienceList(Invalidator::new());
    provide_context(list);
    let store = Store::new(AudienceListData::default());
    let state = list.patched(list_my_audiences, move |rows| store.audiences().patch(rows));

    // The subscriber roster: fetched once (constant source — never refetched, so no sticky
    // retention is needed), derived straight from the resource and provided as a signal so
    // each `MemberChecklist` reflects it reactively when it resolves, without rebuilding rows.
    let subscribers_res = crate::server_resource(|| (), |()| list_my_subscribers());
    let subscribers = Signal::derive(move || {
        subscribers_res
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
    });
    provide_context(subscribers);

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
                    // Mounted unconditionally: never inside a load/error branch that could
                    // tear it down, so only keyed reconciliation ever touches rows.
                    <ul class="j-audience-list">
                        <For each=move || store.audiences() key=|row| row.key() let:row>
                            <AudienceRow row=row.into() />
                        </For>
                    </ul>
                    // Sibling status: loading / empty / error sit next to the list, not
                    // wrapped around it.
                    {move || match state.get() {
                        ListState::Loading => {
                            Some(view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any())
                        }
                        ListState::Empty => Some(view! { <p>"No audiences yet."</p> }.into_any()),
                        ListState::Error(e) => Some(view! { <p class="error">{e}</p> }.into_any()),
                        ListState::Loaded => None,
                    }}
                </section>
            </div>
        </div>
    }
}

/// The "Create an audience" card: owns the create action, which refetches the audience
/// list on a successful create via the `AudienceList` invalidator.
#[component]
fn CreateAudienceForm() -> impl IntoView {
    let create_action = expect_context::<AudienceList>().action::<CreateAudience>();

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
/// author's active subscribers (checked = member). Takes the row's keyed store field, so
/// a rename updates the `<h3>` name in place (the row is never remounted).
#[component]
fn AudienceRow(row: Field<AudienceSummary>) -> impl IntoView {
    let audience_id = row.audience_id().get_untracked();
    let initial_name = row.name().get_untracked();
    view! {
        <li class="j-audience-item">
            <h3 class="j-audience-name">{move || row.name().get()}</h3>
            <AudienceHeader audience_id=audience_id name=initial_name />
            <MemberChecklist audience_id=audience_id />
        </li>
    }
}

/// The `j-audience-head` controls: rename and delete forms for one audience. Both actions
/// refetch the audience list on success via the `AudienceList` invalidator.
#[component]
fn AudienceHeader(audience_id: i64, name: String) -> impl IntoView {
    let list = expect_context::<AudienceList>();
    let rename_action = list.action::<RenameAudience>();
    let delete_action = list.action::<DeleteAudience>();

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

/// Per-subscriber add/remove checklist for one audience. Owns the add/remove actions and
/// the members resource, all bound to a *local* `Invalidator` so a toggle refetches only
/// this audience's members — never the list.
#[component]
fn MemberChecklist(audience_id: i64) -> impl IntoView {
    // The subscriber roster, reactive: it updates the checklist in place when it resolves,
    // without the row being rebuilt (provided by `AudiencesPage`).
    let subscribers = expect_context::<Signal<Vec<SubscriberSummary>>>();
    // Local to this checklist: an add/remove here refetches only this audience's members,
    // not every audience's (and never the list).
    let members = Invalidator::new();
    let add_action = members.action::<AddSubscriberToAudience>();
    let remove_action = members.action::<RemoveSubscriberFromAudience>();

    let members_res = members.resource(move || list_audience_members(audience_id));
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
            let subscribers = subscribers.get();
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

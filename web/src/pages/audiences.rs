//! Named-audience management screen (account area).
//!
//! Lists the author's named audiences with create / rename / delete, and per
//! audience lets the author assign or unassign their active subscribers via a
//! checklist. Mirrors the broad visual conventions of the backup/admin screens
//! (section headings, `j-card` forms, `j-btn` buttons).

use crate::audiences::{
    list_audience_members, list_my_audiences, list_my_subscribers, AddSubscriberToAudience,
    AudienceSummary, CreateAudience, DeleteAudience, RemoveSubscriberFromAudience, RenameAudience,
    SubscriberSummary,
};
use crate::pages::Topbar;
use leptos::prelude::*;

/// Account-area page for managing named audiences and their membership.
#[allow(clippy::must_use_candidate)]
#[component]
pub fn AudiencesPage() -> impl IntoView {
    let create_action = ServerAction::<CreateAudience>::new();
    let rename_action = ServerAction::<RenameAudience>::new();
    let delete_action = ServerAction::<DeleteAudience>::new();
    let add_action = ServerAction::<AddSubscriberToAudience>::new();
    let remove_action = ServerAction::<RemoveSubscriberFromAudience>::new();

    // Re-fetch the audience list and subscriber roster whenever any mutation
    // bumps its version.
    let version = move || {
        (
            create_action.version().get(),
            rename_action.version().get(),
            delete_action.version().get(),
            add_action.version().get(),
            remove_action.version().get(),
        )
    };
    let audiences = Resource::new(version, |_| list_my_audiences());
    let subscribers = Resource::new(version, |_| list_my_subscribers());

    view! {
        <Topbar title="Audiences".to_string() sub="Named subscriber groups".to_string() />
        <div class="j-scroll">
            <div class="j-page">
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

                <section class="j-card">
                    <div class="j-card-head">
                        <div>
                            <h2>"Your audiences"</h2>
                            <div class="j-sub">
                                "Rename, delete, or assign subscribers to each audience."
                            </div>
                        </div>
                    </div>
                    <Suspense fallback=|| {
                        view! { <p class="j-loading">"Loading\u{2026}"</p> }
                    }>
                        {move || Suspend::new(async move {
                            let audiences = audiences.await;
                            let subscribers = subscribers.await.unwrap_or_default();
                            match audiences {
                                Ok(list) if list.is_empty() => {
                                    view! { <p>"No audiences yet."</p> }.into_any()
                                }
                                Ok(list) => {
                                    let subscribers = subscribers.clone();
                                    view! {
                                        <ul class="j-audience-list">
                                            {list
                                                .into_iter()
                                                .map(|audience| {
                                                    view! {
                                                        <AudienceRow
                                                            audience=audience
                                                            subscribers=subscribers.clone()
                                                            rename_action=rename_action
                                                            delete_action=delete_action
                                                            add_action=add_action
                                                            remove_action=remove_action
                                                        />
                                                    }
                                                })
                                                .collect::<Vec<_>>()}
                                        </ul>
                                    }
                                        .into_any()
                                }
                                Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                            }
                        })}
                    </Suspense>
                </section>
            </div>
        </div>
    }
}

/// One audience: its name with rename/delete controls and a checklist of the
/// author's active subscribers (checked = member).
#[allow(clippy::must_use_candidate)]
#[component]
fn AudienceRow(
    audience: AudienceSummary,
    subscribers: Vec<SubscriberSummary>,
    rename_action: ServerAction<RenameAudience>,
    delete_action: ServerAction<DeleteAudience>,
    add_action: ServerAction<AddSubscriberToAudience>,
    remove_action: ServerAction<RemoveSubscriberFromAudience>,
) -> impl IntoView {
    let audience_id = audience.audience_id;
    let name = audience.name.clone();

    // Members re-fetch whenever an assign/unassign mutation lands.
    let members = Resource::new(
        move || (add_action.version().get(), remove_action.version().get()),
        move |_| list_audience_members(audience_id),
    );

    view! {
        <li class="j-audience-item">
            <div class="j-audience-head">
                <ActionForm action=rename_action>
                    <input type="hidden" name="audience_id" value=audience_id />
                    <input type="text" name="name" value=name.clone() />
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
            <Suspense fallback=|| {
                view! { <p class="j-loading">"Loading members\u{2026}"</p> }
            }>
                {move || {
                    let subscribers = subscribers.clone();
                    Suspend::new(async move {
                        let member_ids = members.await.unwrap_or_default();
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
                    })
                }}
            </Suspense>
        </li>
    }
}

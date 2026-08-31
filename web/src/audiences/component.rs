//! The co-located reactive UI for named-audience management: `AudiencesPage` and
//! its child components, plus the keyed reactive store backing the list. Wasm-only.

use super::model::{SubscriberSummary, Summary, SummaryStoreFields};
use super::{
    AddSubscriber, AudienceMembershipRequest, Create, Delete, RemoveSubscriber, Rename,
    RenameAudienceRequest,
};
use crate::error::{WebError, WebResult};
use crate::forms::{self, ValidatedBareInput};
use crate::icon::Icons;
use crate::reactive::{Invalidator, invalidator_scope};
use crate::topbar::Topbar;
use client::reactive;
use common::ids::{AudienceId, SubscriptionId};
use common::list_state::ListState;
use common::{MutationOutcome, audience::AudienceName};
use leptos::prelude::*;
use reactive_stores::{Field, Patch, Store};

/// The reactive store backing the audience list: a keyed collection so a refetch
/// `patch`es row-identically (only changed rows' subfields notify), never remounting
/// unchanged rows. Distinct from `AudienceList` (#359's invalidator scope).
#[derive(Default, Store, Patch)]
struct AudienceListData {
    #[store(key: AudienceId = |a| a.audience_id)]
    audiences: Vec<Summary>,
}

/// The subscriber roster shared via context: a reactive signal over the roster's full
/// resolved state — `None` while loading, `Some(Err)` on a fetch failure, `Some(Ok)`
/// once loaded — so consumers distinguish an error from a genuinely empty roster (#346).
/// Provided by `AudiencesPage`, read by each `MemberChecklist`.
type RosterSignal = Signal<Option<WebResult<Vec<SubscriberSummary>>>>;

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
    // and `patch`ed in place on success (`reactive::patched` owns the plumbing) — so
    // unchanged rows keep their DOM (and their `MemberChecklist`'s loaded members) and a rename
    // updates just that row's name. `state` drives the sibling loading/empty/error node.
    let list = AudienceList(Invalidator::new());
    provide_context(list);
    let store = Store::new(AudienceListData::default());
    let state = reactive::patched(
        move || list.track(),
        super::list_mine,
        move |rows| store.audiences().patch(rows),
    );

    // The subscriber roster: an `Invalidator`-driven `sticky` resource so the refresh
    // control (in the card head below) refetches it while retaining the current roster —
    // flash-free (#347). Provided as a `RosterSignal`: one source of truth for the
    // page-level error node below and each `MemberChecklist`. A fetch error is surfaced,
    // never swallowed into an empty roster (#346).
    let roster = Invalidator::new();
    let subscribers: RosterSignal =
        reactive::sticky(move || roster.track(), super::list_my_subscribers);
    provide_context(subscribers);

    view! {
        <Topbar title="Audiences" sub="Named subscriber groups" />
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
                        // Inline `<svg>` (not `<Icon>`) so the button owns its own
                        // markup; glyph data is shared via `Icons::REFRESH`.
                        <button
                            type="button"
                            class="j-icon-btn"
                            aria-label="Refresh subscribers"
                            on:click=move |_| roster.notify()
                        >
                            <svg
                                class="j-icon"
                                width="16"
                                height="16"
                                viewBox="0 0 20 20"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="1.6"
                                stroke-linecap="round"
                                stroke-linejoin="round"
                            >
                                <path d=Icons::REFRESH />
                            </svg>
                        </button>
                    </div>
                    // Roster fetch error: surfaced once here (the roster feeds every
                    // checklist), mirroring the audience-list error sibling below. Silent
                    // while loading and on success (#346).
                    {move || {
                        subscribers
                            .get()
                            .and_then(Result::err)
                            .map(|e| {
                                view! {
                                    <p class="error">
                                        {format!("Couldn't load your subscribers: {e}")}
                                    </p>
                                }
                            })
                    }}
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
    let list = expect_context::<AudienceList>();
    let create_action = reactive::action::<Create>(move || list.notify());
    // Client-side pre-validation (ADR-0065) via direct-bind: the same `AudienceName::from_str`
    // the typed `#[server]` arg decodes through gates submit (disable-until-valid), so a valid
    // name is a precondition of dispatch and the empty-name rejection never round-trips for a
    // real client. `required` is dropped — the newtype rule is the single source of truth.
    let name = forms::Field::<AudienceName>::new();

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
                <ValidatedBareInput<AudienceName>
                    name="name"
                    field=name
                    placeholder=Some("Audience name")
                />
                <button
                    type="submit"
                    class="j-btn is-primary"
                    prop:disabled=move || !name.is_valid()
                >
                    "Create"
                </button>
            </ActionForm>
            // Touched-gated inline validation message (the newtype's own `Display`).
            {forms::validated_error(
                name.error,
                Signal::derive(move || name.is_touched()),
                |m| view! { <p class="error">{m}</p> }.into_any(),
            )}
            // Server-action error (e.g. a duplicate name).
            {move || match create_action.value().get() {
                Some(Err(error)) => {
                    Some(view! { <p class="error">{error.to_string()}</p> }.into_any())
                }
                Some(Ok(MutationOutcome::CommitIndeterminate(_))) => {
                    Some(
                        view! {
                            <p class="error">
                                "The audience may have been created, but its status could not be confirmed. Refresh to check."
                            </p>
                        }
                            .into_any(),
                    )
                }
                Some(Ok(MutationOutcome::Confirmed(_))) | None => None,
            }}
        </section>
    }
}

/// One audience: its name with rename/delete controls and a checklist of the
/// author's active subscribers (checked = member). Takes the row's keyed store field, so
/// a rename updates the `<h3>` name in place (the row is never remounted).
#[component]
fn AudienceRow(row: Field<Summary>) -> impl IntoView {
    let audience_id = row.audience_id().get_untracked();
    let initial_name = row.name().get_untracked();
    view! {
        <li class="j-audience-item">
            <h3 class="j-audience-name">{move || row.name().get().to_string()}</h3>
            <AudienceHeader audience_id=audience_id name=initial_name />
            <MemberChecklist audience_id=audience_id />
        </li>
    }
}

/// The `j-audience-head` controls: rename and delete forms for one audience. Both actions
/// refetch the audience list on success via the `AudienceList` invalidator.
#[component]
fn AudienceHeader(audience_id: AudienceId, name: AudienceName) -> impl IntoView {
    let list = expect_context::<AudienceList>();
    let rename_action = reactive::action::<Rename>(move || list.notify());
    let delete_action = reactive::action::<Delete>(move || list.notify());
    // Client-side pre-validation (ADR-0065), seeded from the existing name so a pristine
    // row is already valid (submit enabled); clearing it disables Rename and — once
    // touched — shows the newtype's own message inline.
    let name = forms::Field::<AudienceName>::prefilled(&name);
    let (rename_disabled, submit_rename) = forms::server_action_submit(rename_action, move || {
        name.parsed().map(|name| Rename {
            request: RenameAudienceRequest { audience_id, name },
        })
    });

    view! {
        <div class="j-audience-head">
            <form on:submit=submit_rename>
                <ValidatedBareInput<AudienceName> name="name" field=name />
                <button type="submit" class="j-btn" prop:disabled=move || rename_disabled.get()>
                    "Rename"
                </button>
                {forms::validated_error(
                    name.error,
                    Signal::derive(move || name.is_touched()),
                    |m| view! { <p class="error">{m}</p> }.into_any(),
                )}
                {move || match rename_action.value().get() {
                    Some(Err(error)) => {
                        Some(view! { <p class="error">{error.to_string()}</p> }.into_any())
                    }
                    Some(Ok(MutationOutcome::CommitIndeterminate(()))) => {
                        Some(
                            view! {
                                <p class="error">
                                    "The audience may have been renamed, but its status could not be confirmed. Refresh to check."
                                </p>
                            }
                                .into_any(),
                        )
                    }
                    Some(Ok(MutationOutcome::Confirmed(()))) | None => None,
                }}
            </form>
            <ActionForm action=delete_action>
                <input type="hidden" name="audience_id" value=i64::from(audience_id) />
                <button type="submit" class="j-btn is-danger">
                    "Delete"
                </button>
                {move || match delete_action.value().get() {
                    Some(Err(error)) => {
                        Some(view! { <p class="error">{error.to_string()}</p> }.into_any())
                    }
                    Some(Ok(MutationOutcome::CommitIndeterminate(()))) => {
                        Some(
                            view! {
                                <p class="error">
                                    "The audience may have been deleted, but its status could not be confirmed. Refresh to check."
                                </p>
                            }
                                .into_any(),
                        )
                    }
                    Some(Ok(MutationOutcome::Confirmed(()))) | None => None,
                }}
            </ActionForm>
        </div>
    }
}

/// Per-subscriber add/remove checklist for one audience. Owns the add/remove actions and a
/// *local* `Invalidator` whose `sticky` member list refetches only this audience's members
/// on a toggle — never the whole list.
#[component]
fn MemberChecklist(audience_id: AudienceId) -> impl IntoView {
    // The subscriber roster, reactive (provided by `AudiencesPage`): it carries the full
    // resolved state and updates the checklist in place when it resolves, without the row
    // being rebuilt. A fetch error renders nothing here (surfaced once at page level), not
    // an empty roster (#346).
    let subscribers = expect_context::<RosterSignal>();
    // Local to this checklist: an add/remove here refetches only this audience's members,
    // not every audience's (and never the list). `sticky` retains the last member list across
    // that refetch so a toggle never flashes "Loading members…" (`None` until first resolve).
    let members = Invalidator::new();
    let add_action = reactive::action::<AddSubscriber>(move || members.notify());
    let remove_action = reactive::action::<RemoveSubscriber>(move || members.notify());
    let member_ids = reactive::sticky(
        move || members.track(),
        move || super::list_members(audience_id),
    );

    view! {
        {move || {
            match member_ids.get() {
                None => view! { <p class="j-loading">"Loading members\u{2026}"</p> }.into_any(),
                Some(Err(e)) => {
                    // Surface a members fetch error rather than swallowing it into an empty set
                    // (which would misrepresent everyone as a non-member) — consistent with the
                    // audience list (#346). Stringify at the render site: `sticky` preserves
                    // the structured `WebError`, which is `Display` but not `IntoRender` (#347).
                    view! { <p class="error">{e.to_string()}</p> }
                        .into_any()
                }
                Some(Ok(member_ids)) => {
                    let Some(Ok(subscribers)) = subscribers.get() else {
                        return ().into_any();
                    };
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
                                    view! {
                                        <MemberToggle
                                            audience_id=audience_id
                                            subscription_id=sub.subscription_id
                                            label=sub.label
                                            is_member=is_member
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
            }
        }}
    }
}

fn membership_feedback(
    result: Option<Result<MutationOutcome<()>, WebError>>,
    indeterminate_message: &'static str,
) -> Option<AnyView> {
    match result? {
        Ok(MutationOutcome::Confirmed(())) => None,
        Ok(MutationOutcome::CommitIndeterminate(())) => {
            Some(view! { <p class="error">{indeterminate_message}</p> }.into_any())
        }
        Err(error) => Some(view! { <p class="error">{error.to_string()}</p> }.into_any()),
    }
}

/// One subscriber row of a [`MemberChecklist`]: a "Remove" form when the subscriber is
/// already in the audience, an "Add" form when they are not.
///
/// Split out (#306) so the checklist's `view!` carries only the loading/error/empty
/// decisions and this component owns the per-row membership branch.
#[component]
fn MemberToggle(
    audience_id: AudienceId,
    subscription_id: SubscriptionId,
    /// The subscriber's display label (username, or the raw reference when unresolved).
    label: String,
    /// Whether this subscriber is currently in `audience_id`.
    is_member: bool,
    add_action: ServerAction<AddSubscriber>,
    remove_action: ServerAction<RemoveSubscriber>,
) -> impl IntoView {
    let request = AudienceMembershipRequest {
        audience_id,
        subscription_id,
    };
    let remove_request = request.clone();
    let (remove_disabled, submit_remove) = forms::server_action_submit(remove_action, move || {
        Some(RemoveSubscriber {
            request: remove_request.clone(),
        })
    });
    let (add_disabled, submit_add) = forms::server_action_submit(add_action, move || {
        Some(AddSubscriber {
            request: request.clone(),
        })
    });

    view! {
        {if is_member {
            view! {
                <li>
                    <form on:submit=submit_remove>
                        <span class="j-audience-member is-member">{label}</span>
                        <button
                            type="submit"
                            class="j-btn"
                            prop:disabled=move || remove_disabled.get()
                        >
                            "Remove"
                        </button>
                        {move || {
                            membership_feedback(
                                remove_action.value().get(),
                                "The subscriber may have been removed, but its status could not be confirmed. Refresh to check.",
                            )
                        }}
                    </form>
                </li>
            }
                .into_any()
        } else {
            view! {
                <li>
                    <form on:submit=submit_add>
                        <span class="j-audience-member">{label}</span>
                        <button
                            type="submit"
                            class="j-btn"
                            prop:disabled=move || add_disabled.get()
                        >
                            "Add"
                        </button>
                        {move || {
                            membership_feedback(
                                add_action.value().get(),
                                "The subscriber may have been added, but its status could not be confirmed. Refresh to check.",
                            )
                        }}
                    </form>
                </li>
            }
                .into_any()
        }}
    }
}

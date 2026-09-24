//! Authenticated Manage Posts page.

use leptos::prelude::*;
use leptos::task::spawn_local;

use common::pagination::PageSize;

use super::{
    BulkManageOperation, ManageAudienceFilter, ManageConfirmationKind, ManagePageState,
    ManagePublicationState, ManageSelectionIntent, ManageSelectionState, ManagedAudienceTarget,
    ManagedPost, ManagedPostLifecycle, bulk_result_message, delete_count_matches,
    delete_requires_count,
};
use crate::auth;
use crate::topbar::Topbar;

fn lifecycle_label(lifecycle: ManagedPostLifecycle) -> &'static str {
    match lifecycle {
        ManagedPostLifecycle::Draft => "Draft",
        ManagedPostLifecycle::Scheduled => "Scheduled",
        ManagedPostLifecycle::Published => "Published",
    }
}

fn audience_label(audiences: &[ManagedAudienceTarget]) -> String {
    if audiences.is_empty() {
        return "Private".to_owned();
    }
    audiences
        .iter()
        .map(|target| match target {
            ManagedAudienceTarget::Public => "Public".to_owned(),
            ManagedAudienceTarget::Subscribers => "Subscribers".to_owned(),
            ManagedAudienceTarget::Named(id) => format!("Audience {id}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn post_label(post: &ManagedPost) -> String {
    post.title
        .as_ref()
        .map_or_else(|| post.fallback_label.clone().into(), ToString::to_string)
}

fn state_from_value(value: &str) -> ManagePublicationState {
    match value {
        "draft" => ManagePublicationState::Draft,
        "scheduled" => ManagePublicationState::Scheduled,
        "published" => ManagePublicationState::Published,
        _ => ManagePublicationState::All,
    }
}

fn audience_from_value(value: &str) -> ManageAudienceFilter {
    // crap:allow: pure CSR parser has exhaustive unit coverage outside the host LLVM profile
    match value {
        "public" => ManageAudienceFilter::Public,
        "subscribers" => ManageAudienceFilter::Subscribers,
        "private" => ManageAudienceFilter::Private,
        named if named.starts_with("named:") => named
            .trim_start_matches("named:")
            .parse::<i64>()
            .ok()
            .map(common::ids::AudienceId::from)
            .map_or(ManageAudienceFilter::All, ManageAudienceFilter::Named),
        _ => ManageAudienceFilter::All,
    }
}

fn install_load_effects(signals: ManagePageState, session: auth::SessionContext) {
    Effect::new(move |_| {
        spawn_local(async move {
            if matches!(session.reconcile.await, Ok(Some(_)))
                && let Ok(audiences) = crate::audiences::list_mine().await
            {
                signals.named_audiences.set(audiences);
            }
        });
    });
    Effect::new(move |_| {
        let request_state = signals.state.get();
        let request_audience = signals.audience.get();
        let request_search = signals.search.get();
        let request_cursor = signals.cursor.get();
        signals.reload.track();
        signals.loading.set(true);
        signals.error.set(None);
        spawn_local(async move {
            match session.reconcile.await {
                Ok(Some(_)) => match super::list_managed_posts(
                    request_state,
                    request_audience,
                    request_search,
                    request_cursor,
                    Some(PageSize::default()),
                )
                .await
                {
                    Ok(result) => signals.page.set(Some(result)),
                    Err(error) => signals.error.set(Some(error.to_string())),
                },
                Ok(None) => signals
                    .error
                    .set(Some("Sign in to manage Posts.".to_owned())),
                Err(error) => signals.error.set(Some(error.to_string())),
            }
            signals.loading.set(false);
        });
    });
}

#[component]
fn ManagedPostRow(
    post: ManagedPost,
    selection: RwSignal<ManageSelectionState>,
    disabled: Signal<bool>,
) -> impl IntoView {
    let post_for_toggle = post.clone();
    let post_id = post.post_id;
    let label = post_label(&post);
    let audience = audience_label(&post.audiences);
    view! {
        <li class="j-manage-post" data-test="managed-post" data-post-id=post_id.to_string()>
            <label class="j-manage-select">
                <input
                    type="checkbox"
                    aria-label=format!("Select {label}")
                    prop:checked=move || selection.with(|current| current.is_selected(post_id))
                    disabled=move || disabled.get()
                    on:change=move |_| selection.update(|current| current.toggle(&post_for_toggle))
                />
                <span class="j-manage-title">{label}</span>
            </label>
            <span class="j-badge">{lifecycle_label(post.lifecycle)}</span>
            <span class="j-manage-audience">{audience}</span>
            <time datetime=post
                .updated_at
                .to_string()>{super::render::format_post_time(post.updated_at)}</time>
            <span class="j-manage-actions">
                <a href=format!("/posts/{post_id}/edit")>"Edit"</a>
                " · "
                <a href=format!("/posts/{post_id}/history")>"History"</a>
            </span>
        </li>
    }
}

#[component]
fn ManageFilters(signals: ManagePageState) -> impl IntoView {
    let apply = move |()| {
        signals.search.set(signals.search_input.get());
        signals.cursor.set(None);
        signals.selection.update(ManageSelectionState::clear);
        signals.success.set(None);
    };
    view! {
        <form
            class="j-panel j-manage-filters"
            on:submit=move |ev| {
                ev.prevent_default();
                apply(());
            }
        >
            <label class="j-form-field">
                <span class="j-form-label">"Search title or slug"</span>
                <input
                    class="j-form-input"
                    type="search"
                    value=move || signals.search_input.get()
                    on:input=move |ev| signals.search_input.set(event_target_value(&ev))
                />
            </label>
            <label class="j-form-field">
                <span class="j-form-label">"State"</span>
                <select
                    class="j-form-input"
                    on:change=move |ev| {
                        signals.state.set(state_from_value(&event_target_value(&ev)));
                        signals.cursor.set(None);
                    }
                >
                    <option value="all">"All"</option>
                    <option value="draft">"Draft"</option>
                    <option value="scheduled">"Scheduled"</option>
                    <option value="published">"Published"</option>
                </select>
            </label>
            <label class="j-form-field">
                <span class="j-form-label">"Audience"</span>
                <select
                    class="j-form-input"
                    on:change=move |ev| {
                        signals.audience.set(audience_from_value(&event_target_value(&ev)));
                        signals.cursor.set(None);
                    }
                >
                    <option value="all">"All"</option>
                    <option value="public">"Public"</option>
                    <option value="subscribers">"Subscribers"</option>
                    <option value="private">"Private"</option>
                    <For
                        each=move || signals.named_audiences.get()
                        key=|named| named.audience_id
                        children=move |named| {
                            view! {
                                <option value=format!(
                                    "named:{}",
                                    named.audience_id,
                                )>{format!("Named: {}", named.name)}</option>
                            }
                        }
                    />
                </select>
            </label>
            <button class="j-btn is-primary" type="submit">
                "Apply filters"
            </button>
        </form>
    }
}

fn select_all_matching(signals: ManagePageState) {
    signals.pending.set(true);
    signals.error.set(None);
    let intent = ManageSelectionIntent::AllMatching {
        state: signals.state.get(),
        audience: signals.audience.get(),
        search: signals.search.get(),
    };
    spawn_local(async move {
        match super::resolve_management_selection(intent).await {
            Ok(snapshot) => signals
                .selection
                .update(|current| current.replace_with_snapshot(&snapshot)),
            Err(error) => signals.error.set(Some(error.to_string())),
        }
        signals.pending.set(false);
    });
}

fn open_confirmation(signals: ManagePageState, kind: ManageConfirmationKind) {
    let ids = signals.selection.with(ManageSelectionState::post_ids);
    if ids.is_empty() {
        return;
    }
    signals.pending.set(true);
    signals.error.set(None);
    spawn_local(async move {
        match super::resolve_management_selection(ManageSelectionIntent::Explicit { post_ids: ids })
            .await
        {
            Ok(snapshot) => {
                signals
                    .selection
                    .update(|current| current.replace_with_snapshot(&snapshot));
                signals.confirmation.set(Some((kind, snapshot)));
                signals.delete_count.set(String::new());
            }
            Err(error) => signals.error.set(Some(error.to_string())),
        }
        signals.pending.set(false);
    });
}

#[component]
fn ManageToolbar(signals: ManagePageState) -> impl IntoView {
    let disabled = move || signals.pending.get() || signals.confirmation.get().is_some();
    let none_selected = move || {
        signals
            .selection
            .with(|current| current.selected_count() == 0)
    };
    view! {
        <section class="j-panel j-manage-toolbar" aria-label="Bulk actions">
            <strong>
                {move || {
                    format!(
                        "{} selected",
                        signals.selection.with(ManageSelectionState::selected_count),
                    )
                }}
            </strong>
            <button
                class="j-btn"
                type="button"
                disabled=move || signals.pending.get()
                on:click=move |_| select_all_matching(signals)
            >
                "Select all matching"
            </button>
            <button
                class="j-btn"
                type="button"
                disabled=disabled
                on:click=move |_| signals.selection.update(ManageSelectionState::clear)
            >
                "Clear"
            </button>
            <button
                class="j-btn"
                type="button"
                disabled=move || disabled() || none_selected()
                on:click=move |_| open_confirmation(signals, ManageConfirmationKind::Audience)
            >
                "Change audience"
            </button>
            <button
                class="j-btn is-danger"
                type="button"
                disabled=move || disabled() || none_selected()
                on:click=move |_| open_confirmation(signals, ManageConfirmationKind::Delete)
            >
                "Delete"
            </button>
        </section>
    }
}

#[component]
fn ManagePostList(signals: ManagePageState) -> impl IntoView {
    let disabled =
        Signal::derive(move || signals.pending.get() || signals.confirmation.get().is_some());
    view! {
        <Show when=move || signals.loading.get()>
            <p class="j-loading">"Loading\u{2026}"</p>
        </Show>
        <ul class="j-manage-list" aria-label="Posts">
            <For
                each=move || signals.page.get().map_or_else(Vec::new, |page| page.posts)
                key=|post| post.post_id
                children=move |post| {
                    view! {
                        <ManagedPostRow post=post selection=signals.selection disabled=disabled />
                    }
                }
            />
        </ul>
        <Show when=move || {
            !signals.loading.get() && signals.page.get().is_some_and(|page| page.posts.is_empty())
        }>
            <p class="j-sub">"No Posts match these filters."</p>
        </Show>
        <Show when=move || signals.page.get().is_some_and(|page| page.has_more)>
            <button
                class="j-btn"
                type="button"
                disabled=move || disabled.get()
                on:click=move |_| {
                    signals.cursor.set(signals.page.get().and_then(|page| page.next_cursor));
                }
            >
                "Next page"
            </button>
        </Show>
    }
}

fn confirmation_operation(
    kind: ManageConfirmationKind,
    selected_count: usize,
    delete_count: &str,
    audience: common::visibility::AudienceSelection,
) -> Option<BulkManageOperation> {
    if kind == ManageConfirmationKind::Delete && !delete_count_matches(selected_count, delete_count)
    {
        return None;
    }
    Some(match kind {
        ManageConfirmationKind::Audience => BulkManageOperation::ChangeAudience { audience },
        ManageConfirmationKind::Delete => BulkManageOperation::Delete {
            confirmed_count: delete_count.trim().parse().ok(),
        },
    })
}

fn execute_confirmation(signals: ManagePageState) {
    // crap:allow: reactive server-function outcomes are exercised by manage-posts E2E
    let Some((kind, snapshot)) = signals.confirmation.get() else {
        return;
    };
    let Some(operation) = confirmation_operation(
        kind,
        snapshot.selected_count,
        &signals.delete_count.get(),
        signals.replacement.get(),
    ) else {
        return;
    };
    signals.pending.set(true);
    signals.error.set(None);
    signals.success.set(None);
    // cov:ignore-start: asynchronous browser orchestration is covered by manage-posts E2E
    spawn_local(async move {
        match super::execute_management_operation(snapshot, operation).await {
            Ok(common::MutationOutcome::Confirmed(result)) => {
                signals.success.set(Some(bulk_result_message(result)));
                signals.selection.update(ManageSelectionState::clear);
                signals.confirmation.set(None);
                signals.reload.update(|value| *value += 1);
            }
            Ok(common::MutationOutcome::CommitIndeterminate(_)) => signals.error.set(Some(
                "Commit confirmation was lost; refresh before retrying.".to_owned(),
            )),
            Err(error) => signals.error.set(Some(error.to_string())),
        }
        signals.pending.set(false);
    });
    // cov:ignore-stop
}

#[component]
fn ManageConfirmation(signals: ManagePageState) -> impl IntoView {
    view! {
        <Show when=move || signals.confirmation.get().is_some()>
            <section
                class="j-panel j-manage-confirm"
                role="dialog"
                aria-modal="true"
                aria-labelledby="bulk-confirm-title"
            >
                <h2 id="bulk-confirm-title">
                    {move || match signals.confirmation.get().map(|(kind, _)| kind) {
                        Some(ManageConfirmationKind::Audience) => "Confirm audience replacement",
                        Some(ManageConfirmationKind::Delete) => "Confirm Post deletion",
                        None => "Confirm bulk operation",
                    }}
                </h2>
                <p>
                    {move || {
                        format!(
                            "This operation targets exactly {} Posts.",
                            signals
                                .confirmation
                                .get()
                                .map_or(0, |(_, snapshot)| snapshot.selected_count),
                        )
                    }}
                </p>
                <Show when=move || {
                    signals
                        .confirmation
                        .get()
                        .is_some_and(|(kind, _)| kind == ManageConfirmationKind::Audience)
                }>
                    <super::AudiencePicker selection=signals.replacement />
                    <p class="j-sub">
                        "The complete Audience Selection above replaces every selected Post's current audience."
                    </p>
                </Show>
                <Show when=move || {
                    signals
                        .confirmation
                        .get()
                        .is_some_and(|(kind, snapshot)| {
                            kind == ManageConfirmationKind::Delete
                                && delete_requires_count(snapshot.selected_count)
                        })
                }>
                    <label class="j-form-field">
                        <span class="j-form-label">
                            {move || {
                                format!(
                                    "Enter {} to confirm",
                                    signals
                                        .confirmation
                                        .get()
                                        .map_or(0, |(_, snapshot)| snapshot.selected_count),
                                )
                            }}
                        </span>
                        <input
                            class="j-form-input"
                            inputmode="numeric"
                            value=move || signals.delete_count.get()
                            on:input=move |ev| signals.delete_count.set(event_target_value(&ev))
                        />
                    </label>
                </Show>
                <div class="j-actions">
                    <button
                        class="j-btn is-primary"
                        type="button"
                        disabled=move || {
                            signals.pending.get()
                                || signals
                                    .confirmation
                                    .get()
                                    .is_some_and(|(kind, snapshot)| {
                                        kind == ManageConfirmationKind::Delete
                                            && !delete_count_matches(
                                                snapshot.selected_count,
                                                &signals.delete_count.get(),
                                            )
                                    })
                        }
                        on:click=move |_| execute_confirmation(signals)
                    >
                        {move || if signals.pending.get() { "Working\u{2026}" } else { "Confirm" }}
                    </button>
                    <button
                        class="j-btn"
                        type="button"
                        disabled=move || signals.pending.get()
                        on:click=move |_| signals.confirmation.set(None)
                    >
                        "Cancel"
                    </button>
                </div>
            </section>
        </Show>
    }
}

#[component]
fn ManageFeedback(signals: ManagePageState) -> impl IntoView {
    view! {
        <Show when=move || signals.success.get().is_some()>
            <p class="success" role="status">
                {move || signals.success.get().unwrap_or_default()}
            </p>
        </Show>
        <Show when=move || signals.error.get().is_some()>
            <p class="error" role="alert">
                {move || signals.error.get().unwrap_or_default()}
            </p>
        </Show>
    }
}

/// Compact, bounded owner-only management workspace.
#[component]
pub fn ManagePostsPage() -> impl IntoView {
    let signals = ManagePageState::default();
    install_load_effects(signals, auth::use_session());
    view! {
        <Topbar title="Manage Posts" sub="Filter, select, and update Posts" />
        <div class="j-scroll">
            <main class="j-page j-manage-page" data-test="manage-posts-page">
                <ManageFilters signals=signals />
                <ManageToolbar signals=signals />
                <ManageFeedback signals=signals />
                <ManagePostList signals=signals />
                <ManageConfirmation signals=signals />
            </main>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::visibility::AudienceSelection;

    #[test]
    fn audience_filter_values_cover_builtins_named_and_invalid_input() {
        assert_eq!(audience_from_value("public"), ManageAudienceFilter::Public);
        assert_eq!(
            audience_from_value("subscribers"),
            ManageAudienceFilter::Subscribers
        );
        assert_eq!(
            audience_from_value("private"),
            ManageAudienceFilter::Private
        );
        assert_eq!(
            audience_from_value("named:7"),
            ManageAudienceFilter::Named(common::ids::AudienceId::from(7))
        );
        assert_eq!(audience_from_value("named:nope"), ManageAudienceFilter::All);
        assert_eq!(audience_from_value("unknown"), ManageAudienceFilter::All);
    }

    #[test]
    fn confirmation_operation_enforces_delete_count_and_preserves_audience() {
        let audience = AudienceSelection {
            public: true,
            subscribers: false,
            named: Vec::new(),
        };
        assert_eq!(
            confirmation_operation(ManageConfirmationKind::Audience, 20, "", audience.clone()),
            Some(BulkManageOperation::ChangeAudience { audience })
        );
        assert_eq!(
            confirmation_operation(
                ManageConfirmationKind::Delete,
                10,
                "9",
                AudienceSelection::default(),
            ),
            None
        );
        assert_eq!(
            confirmation_operation(
                ManageConfirmationKind::Delete,
                10,
                " 10 ",
                AudienceSelection::default(),
            ),
            Some(BulkManageOperation::Delete {
                confirmed_count: Some(10),
            })
        );
        assert_eq!(
            confirmation_operation(
                ManageConfirmationKind::Delete,
                2,
                "",
                AudienceSelection::default(),
            ),
            Some(BulkManageOperation::Delete {
                confirmed_count: None,
            })
        );
    }
}

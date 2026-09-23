use leptos::prelude::*;

use crate::audiences;
use crate::posts::NamedAudienceState;
use common::visibility::AudienceSelection;

/// Start the named-audience load and project every resource outcome into the
/// explicit host-tested state consumed by both the picker and its submit gate.
pub(super) fn load_named_audiences() -> RwSignal<NamedAudienceState> {
    let state = RwSignal::new(NamedAudienceState::Loading);
    let named = Resource::new(|| (), |()| audiences::list_mine());
    Effect::new(move |_| {
        state.set(NamedAudienceState::resolve(named.get()));
    });
    state
}

/// Per-post visibility control for the editor.
///
/// Every checked built-in or named target is retained, even when Public
/// currently dominates visibility. No checked targets means Private.
#[component]
pub fn AudiencePicker(selection: RwSignal<AudienceSelection>) -> impl IntoView {
    let named = load_named_audiences();
    view! { <AudiencePickerWithState selection=selection named=named on_user_change=None /> }
}

/// The picker view over a load state shared with its owning action gate.
#[component]
pub(super) fn AudiencePickerWithState(
    selection: RwSignal<AudienceSelection>,
    named: RwSignal<NamedAudienceState>,
    on_user_change: Option<Callback<()>>,
) -> impl IntoView {
    let is_private = move || {
        selection
            .with(|current| !current.public && !current.subscribers && current.named.is_empty())
    };
    view! {
        <fieldset class="j-form-field j-composer-group j-audience-picker" aria-label="Audience">
            <legend class="j-form-label">"Audience"</legend>
            <Show when=is_private>
                <p class="j-form-help">"Private — only you can see this Post."</p>
            </Show>
            <label class="j-audience-choice" for="audience-public">
                <input
                    id="audience-public"
                    type="checkbox"
                    prop:checked=move || selection.get().public
                    on:change=move |ev| {
                        selection.update(|sel| sel.public = event_target_checked(&ev));
                        notify_user_change(on_user_change);
                    }
                />
                "Public"
            </label>
            <label class="j-audience-choice" for="audience-subscribers">
                <input
                    id="audience-subscribers"
                    type="checkbox"
                    prop:checked=move || selection.get().subscribers
                    on:change=move |ev| {
                        selection.update(|sel| sel.subscribers = event_target_checked(&ev));
                        notify_user_change(on_user_change);
                    }
                />
                "Subscribers"
            </label>
            <NamedAudienceOptions named=named selection=selection on_user_change />
            <button
                class="j-audience-clear"
                type="button"
                on:click=move |_| {
                    selection.set(AudienceSelection::default());
                    notify_user_change(on_user_change);
                }
            >
                "Clear all"
            </button>
        </fieldset>
    }
}

/// Record an intentional picker interaction, independent of its resulting value.
fn notify_user_change(on_user_change: Option<Callback<()>>) {
    if let Some(notify) = on_user_change {
        notify.run(());
    }
}

/// Loading, failure, or successfully loaded named-audience options.
#[component]
fn NamedAudienceOptions(
    named: RwSignal<NamedAudienceState>,
    selection: RwSignal<AudienceSelection>,
    on_user_change: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <Show
            when=move || named.with(|state| matches!(state, NamedAudienceState::Loading))
            fallback=move || {
                view! {
                    <Show
                        when=move || named.with(|state| matches!(state, NamedAudienceState::Failed))
                        fallback=move || {
                            view! {
                                <ReadyNamedAudienceOptions
                                    named=named
                                    selection=selection
                                    on_user_change
                                />
                            }
                        }
                    >
                        <p class="error">"Could not load named audiences."</p>
                    </Show>
                }
            }
        >
            <p class="j-loading">"Loading\u{2026}"</p>
        </Show>
    }
}

/// A successful named-audience load, split between genuine empty and rows.
#[component]
fn ReadyNamedAudienceOptions(
    named: RwSignal<NamedAudienceState>,
    selection: RwSignal<AudienceSelection>,
    on_user_change: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <Show
            when=move || {
                named
                    .with(|state| {
                        matches!(
                            state,
                            NamedAudienceState::Ready(audiences)
                            if audiences.is_empty()
                        )
                    })
            }
            fallback=move || {
                view! { <NamedAudienceRows named=named selection=selection on_user_change /> }
            }
        >
            <p class="j-sub">"No named audiences."</p>
        </Show>
    }
}

/// Checkbox rows for a successfully loaded, non-empty named-audience list.
#[component]
fn NamedAudienceRows(
    named: RwSignal<NamedAudienceState>,
    selection: RwSignal<AudienceSelection>,
    on_user_change: Option<Callback<()>>,
) -> impl IntoView {
    let audiences = move || {
        named.with(|state| match state {
            NamedAudienceState::Ready(audiences) => audiences.clone(),
            NamedAudienceState::Loading | NamedAudienceState::Failed => Vec::new(),
        })
    };

    view! {
        <div class="j-audience-named">
            <span class="j-form-label">"Also share with"</span>
            <For
                each=audiences
                key=|audience| audience.audience_id
                children=move |audience| audience_checkbox(audience, selection, on_user_change)
            />
        </div>
    }
}

/// One named-audience checkbox row for [`AudiencePicker`]. Toggling it
/// adds/removes the audience id in the shared selection.
fn audience_checkbox(
    audience: audiences::Summary,
    selection: RwSignal<AudienceSelection>,
    on_user_change: Option<Callback<()>>,
) -> impl IntoView {
    let id = audience.audience_id;
    let input_id = format!("audience-named-{id}");
    let checked = move || selection.get().named.contains(&id);
    view! {
        <label class="j-audience-choice" for=input_id.clone()>
            <input
                id=input_id.clone()
                type="checkbox"
                prop:checked=checked
                on:change=move |ev| {
                    let on = event_target_checked(&ev);
                    selection
                        .update(|sel| {
                            sel.named.retain(|x| *x != id);
                            if on {
                                sel.named.push(id);
                            }
                        });
                    notify_user_change(on_user_change);
                }
            />
            " "
            {String::from(audience.name)}
        </label>
    }
}

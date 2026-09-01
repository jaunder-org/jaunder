use leptos::prelude::*;

use crate::audiences;
use crate::posts::NamedAudienceState;
use common::visibility::{AudienceBase, AudienceSelection};

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
/// Drives a shared `selection` signal: a mutually-exclusive base
/// (Public / Private / Subscribers) plus a checkbox per named audience the
/// author owns (union semantics — e.g. Public + a named audience). `Private`
/// is author-only and the storage layer drops any named selection for it
/// (see `audience_selection_to_targets`); the named checkboxes are disabled
/// while Private is chosen to make that explicit.
#[component]
pub fn AudiencePicker(selection: RwSignal<AudienceSelection>) -> impl IntoView {
    let named = load_named_audiences();
    view! { <AudiencePickerWithState selection=selection named=named /> }
}

/// The picker view over a load state shared with its owning action gate.
#[component]
pub(super) fn AudiencePickerWithState(
    selection: RwSignal<AudienceSelection>,
    named: RwSignal<NamedAudienceState>,
) -> impl IntoView {
    let change_base = move |ev| {
        if let Ok(base) = AudienceBase::try_from(event_target_value(&ev).as_str()) {
            selection.update(|current| current.base = base);
        }
    };

    view! {
        <div class="j-field-row" style="grid-template-columns:auto 1fr">
            <label class="j-field-label" for="audience-base">
                "Audience"
            </label>
            <select id="audience-base" class="j-field-val" on:change=change_base>
                <For
                    // Each base variant is paired with its caption here, so the
                    // values and visible order cannot drift apart.
                    each=|| {
                        [
                            (AudienceBase::Public, "Public"),
                            (AudienceBase::Subscribers, "Subscribers"),
                            (AudienceBase::Private, "Private (only me)"),
                        ]
                    }
                    key=|(base, _)| *base
                    children=move |(base, label)| {
                        view! {
                            <option
                                value=base.to_string()
                                selected=move || selection.get().base == base
                            >
                                {label}
                            </option>
                        }
                    }
                />
            </select>
        </div>
        <NamedAudienceOptions named=named selection=selection />
    }
}

/// Loading, failure, or successfully loaded named-audience options.
#[component]
fn NamedAudienceOptions(
    named: RwSignal<NamedAudienceState>,
    selection: RwSignal<AudienceSelection>,
) -> impl IntoView {
    view! {
        <Show
            when=move || named.with(|state| matches!(state, NamedAudienceState::Loading))
            fallback=move || {
                view! {
                    <Show
                        when=move || named.with(|state| matches!(state, NamedAudienceState::Failed))
                        fallback=move || {
                            view! { <ReadyNamedAudienceOptions named=named selection=selection /> }
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
                view! { <NamedAudienceRows named=named selection=selection /> }
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
) -> impl IntoView {
    let audiences = move || {
        named.with(|state| match state {
            NamedAudienceState::Ready(audiences) => audiences.clone(),
            NamedAudienceState::Loading | NamedAudienceState::Failed => Vec::new(),
        })
    };

    view! {
        <div style="margin-top:8px">
            <span class="j-field-label">"Also share with"</span>
            <For
                each=audiences
                key=|audience| audience.audience_id
                children=move |audience| audience_checkbox(audience, selection)
            />
        </div>
    }
}

/// One named-audience checkbox row for [`AudiencePicker`]. Toggling it
/// adds/removes the audience id in the shared selection. Disabled while the
/// base is `Private`, since Private cannot combine with named audiences.
fn audience_checkbox(
    audience: audiences::Summary,
    selection: RwSignal<AudienceSelection>,
) -> impl IntoView {
    let id = audience.audience_id;
    let input_id = format!("audience-named-{id}");
    let checked = move || selection.get().named.contains(&id);
    let disabled = move || selection.get().base == AudienceBase::Private;
    view! {
        <label style="display:block" for=input_id.clone()>
            <input
                id=input_id.clone()
                type="checkbox"
                prop:checked=checked
                disabled=disabled
                on:change=move |ev| {
                    let on = event_target_checked(&ev);
                    selection
                        .update(|sel| {
                            sel.named.retain(|x| *x != id);
                            if on {
                                sel.named.push(id);
                            }
                        });
                }
            />
            " "
            {String::from(audience.name)}
        </label>
    }
}

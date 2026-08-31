use crate::error::WebError;
use crate::forms::{Field, ValidatedInput, ValidatedTextarea};
use crate::topbar::Topbar;
use common::{MutationOutcome, bio::Bio, display_name::DisplayName, render::PostFormat};
use leptos::prelude::*;

use super::DefaultPostFormatState;
use super::api::{self, SetDefaultPostFormat, Update};

/// Profile page — shows username, display name, bio; allows updating.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<Update>::new();
    let profile = Resource::new(move || update_action.version().get(), |_| api::get());
    // Client-validated display name and bio (both optional: empty clears them),
    // owned by the component so the bespoke form can `.dispatch` the typed
    // `Update` args — the ADR-0065 direct-bind pattern (mirrors the post
    // compose/edit forms); an `<ActionForm>`'s string fields cannot carry
    // validated `Option<DisplayName>`/`Option<Bio>`.
    let dn_field = Field::<DisplayName>::optional();
    let bio_field = Field::<Bio>::optional();

    view! {
        <Topbar title="Profile" sub="Your details" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match profile.await {
                            Ok(data) => {
                                dn_field
                                    .value
                                    .set(
                                        data.display_name.as_deref().unwrap_or_default().to_string(),
                                    );
                                bio_field
                                    .value
                                    .set(data.bio.as_deref().unwrap_or_default().to_string());
                                let submit = move |_| {
                                    update_action
                                        .dispatch(Update {
                                            display_name: dn_field.parsed(),
                                            bio: bio_field.parsed(),
                                        });
                                };
                                // Seed the form from the persisted profile. This re-runs
                                // (re-seeding) whenever a successful update bumps the
                                // resource; a stored display name is always valid, so the
                                // optional field stays valid.
                                view! {
                                    <div class="j-card">
                                        <div class="j-card-head">
                                            <div>
                                                <h2>"Profile"</h2>
                                                <div class="j-sub">"Your display name and bio."</div>
                                            </div>
                                        </div>
                                        <div class="j-form-body">
                                            <p>"Username: " {data.username.to_string()}</p>
                                            <ValidatedInput<DisplayName>
                                                label="Display Name"
                                                name="display_name"
                                                field=dn_field
                                            />
                                            <ValidatedTextarea<Bio>
                                                label="Bio"
                                                name="bio"
                                                field=bio_field
                                            />
                                        </div>
                                        <div class="j-form-actions">
                                            <button
                                                type="button"
                                                class="j-btn is-primary"
                                                prop:disabled=move || {
                                                    !dn_field.is_valid() || !bio_field.is_valid()
                                                }
                                                on:click=submit
                                            >
                                                "Update Profile"
                                            </button>
                                        </div>
                                    </div>
                                    <DefaultPostFormatControl />
                                }
                                    .into_any()
                            }
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
                {move || {
                    update_action
                        .value()
                        .get()
                        .and_then(|result: Result<MutationOutcome<()>, WebError>| match result {
                            Ok(MutationOutcome::Confirmed(())) => None,
                            Ok(MutationOutcome::CommitIndeterminate(())) => {
                                Some(
                                    view! {
                                        <p class="error">
                                            "Your profile may have been updated, but its status could not be confirmed. Refresh to check."
                                        </p>
                                    }
                                        .into_any(),
                                )
                            }
                            Err(error) => {
                                Some(view! { <p class="error">{error.to_string()}</p> }.into_any())
                            }
                        })
                }}
            </div>
        </div>
    }
}

/// Control for setting the user's default post format preference.
///
/// ADR-0065 direct-bind: a [`DefaultPostFormatState`] signal owned by the
/// component is seeded only by the persisted preference, bound to a `<select>`
/// whose `on:change` parses the token, and read by a plain `type="button"` "Save"
/// that `.dispatch`es the typed `SetDefaultPostFormat` action — no
/// `<ActionForm>` / string-submit path.
#[component]
fn DefaultPostFormatControl() -> impl IntoView {
    let action = ServerAction::<SetDefaultPostFormat>::new();
    let initial = Resource::new(|| (), |()| api::get_default_post_format());
    // The state belongs to the component rather than the transient Suspend
    // scope. Loading and Failed deliberately carry no format, so neither can
    // dispatch a fabricated Markdown preference.
    let state = RwSignal::new(DefaultPostFormatState::Loading);
    let save = move |_| {
        if let Some(format) = state.get().format_to_save() {
            action.dispatch(SetDefaultPostFormat { format });
        }
    };

    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                let resolved = initial.await;
                state.set(DefaultPostFormatState::resolve(Some(&resolved)));
                view! {
                    <div class="j-card">
                        <div class="j-card-head">
                            <div>
                                <h2>"Default Post Format"</h2>
                                <div class="j-sub">"The editor format new posts start in."</div>
                            </div>
                        </div>
                        <div class="j-form-body">
                            <DefaultPostFormatBody state=state />
                        </div>
                        {move || {
                            action
                                .value()
                                .get()
                                .and_then(|result: Result<MutationOutcome<()>, WebError>| match result {
                                    Ok(MutationOutcome::Confirmed(())) => None,
                                    Ok(MutationOutcome::CommitIndeterminate(())) => {
                                        Some(
                                            "Save acknowledgement was lost; reload to verify the default post format."
                                                .to_owned(),
                                        )
                                    }
                                    Err(error) => Some(error.to_string()),
                                })
                                .map(|error| view! { <p class="error">{error}</p> })
                        }}
                        <div class="j-form-actions">
                            <button
                                type="button"
                                class="j-btn"
                                prop:disabled=move || state.get().format_to_save().is_none()
                                on:click=save
                            >
                                "Save"
                            </button>
                        </div>
                    </div>
                }
            })}
        </Suspense>
    }
}

/// The resolved preference body: loading, explicit failure, or the real select.
#[component]
fn DefaultPostFormatBody(state: RwSignal<DefaultPostFormatState>) -> impl IntoView {
    view! {
        <Show
            when=move || matches!(state.get(), DefaultPostFormatState::Ready(_))
            fallback=move || {
                view! {
                    <Show
                        when=move || matches!(state.get(), DefaultPostFormatState::Loading)
                        fallback=|| {
                            view! { <p class="error">"Could not load the default post format."</p> }
                        }
                    >
                        <p class="j-loading">"Loading\u{2026}"</p>
                    </Show>
                }
            }
        >
            <DefaultPostFormatSelect state=state />
        </Show>
    }
}

/// The ready preference select, derived from user-selectable [`PostFormat`] variants.
#[component]
fn DefaultPostFormatSelect(state: RwSignal<DefaultPostFormatState>) -> impl IntoView {
    use strum::{EnumMessage, VariantArray};

    let change = move |ev| {
        if let Ok(format) = event_target_value(&ev).parse::<PostFormat>() {
            state.set(DefaultPostFormatState::Ready(format));
        }
    };

    view! {
        <label class="j-form-field">
            <span class="j-form-label">"Default post format"</span>
            <select id="default-post-format" class="j-form-input" on:change=change>
                <For
                    each=move || {
                        PostFormat::VARIANTS
                            .iter()
                            .copied()
                            .filter_map(|format| {
                                format.get_message().map(|label| (format, label))
                            })
                    }
                    key=|(format, _)| *format
                    children=move |(format, label)| {
                        view! {
                            <option
                                value=format.to_string()
                                selected=move || state.get().format_to_save() == Some(format)
                            >
                                {label}
                            </option>
                        }
                    }
                />
            </select>
        </label>
    }
}

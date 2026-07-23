use crate::error::WebError;
use crate::forms::Field;
use crate::topbar::Topbar;
use common::bio::Bio;
use common::display_name::DisplayName;
use common::render::PostFormat;
use leptos::prelude::*;

use super::api::{get_default_post_format, get_profile, SetDefaultPostFormat, UpdateProfile};

/// Profile page — shows username, display name, bio; allows updating.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<UpdateProfile>::new();
    let profile = Resource::new(move || update_action.version().get(), |_| get_profile());
    // Client-validated display name and bio (both optional: empty clears them),
    // owned by the component so the bespoke form can `.dispatch` the typed
    // `UpdateProfile` args — the ADR-0065 direct-bind pattern (mirrors the post
    // compose/edit forms), replacing the former `<ActionForm>` whose string fields
    // could not carry validated `Option<DisplayName>`/`Option<Bio>`.
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
                                        .dispatch(UpdateProfile {
                                            display_name: dn_field.parsed(),
                                            bio: bio_field.parsed(),
                                        });
                                };
                                // Seed the form from the persisted profile. This re-runs
                                // (re-seeding) whenever a successful update bumps the
                                // resource; a stored display name is always valid, so the
                                // optional field stays valid.
                                view! {
                                    <p>"Username: " {data.username.to_string()}</p>
                                    <label>
                                        "Display Name"
                                        <input
                                            type="text"
                                            name="display_name"
                                            prop:value=dn_field.value
                                            on:input=move |ev| {
                                                let v = event_target_value(&ev);
                                                dn_field.value.set(v.clone());
                                                dn_field.error.set(dn_field.error_for(&v));
                                            }
                                            on:blur=move |_| dn_field.touch()
                                        />
                                    </label>
                                    {move || {
                                        dn_field
                                            .is_touched()
                                            .then(|| dn_field.error.get())
                                            .flatten()
                                            .map(|msg| view! { <p class="error">{msg}</p> })
                                    }}
                                    <label>
                                        "Bio"
                                        <textarea
                                            name="bio"
                                            prop:value=bio_field.value
                                            on:input=move |ev| {
                                                let v = event_target_value(&ev);
                                                bio_field.value.set(v.clone());
                                                bio_field.error.set(bio_field.error_for(&v));
                                            }
                                            on:blur=move |_| bio_field.touch()
                                        />
                                    </label>
                                    {move || {
                                        bio_field
                                            .is_touched()
                                            .then(|| bio_field.error.get())
                                            .flatten()
                                            .map(|msg| view! { <p class="error">{msg}</p> })
                                    }}
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
                        .and_then(|r: Result<(), WebError>| r.err())
                        .map(|e| view! { <p class="error">{e.to_string()}</p> })
                }}
            </div>
        </div>
    }
}

/// Control for setting the user's default post format preference.
///
/// ADR-0065 direct-bind: a `RwSignal<PostFormat>` owned by the component, seeded
/// from the persisted preference, bound to a `<select>` whose `on:change` parses
/// the token, and a plain `type="button"` "Save" that `.dispatch`es the typed
/// `SetDefaultPostFormat` action — no `<ActionForm>` / string-submit path.
///
/// The offered formats are **derived from the `PostFormat` type**, not hard-coded:
/// `PostFormat::VARIANTS` filtered to those carrying a `strum` editor message
/// (`get_message`) — the same source of truth as the editor `FormatToggle`
/// (`posts::FormatToggle`). `Html` is renderer-internal (#445), carries no message,
/// and so is excluded here too; the default falls back to `Markdown` to match.
#[component]
fn DefaultPostFormatControl() -> impl IntoView {
    use strum::{EnumMessage, VariantArray};
    let action = ServerAction::<SetDefaultPostFormat>::new();
    let initial = Resource::new(|| (), |()| get_default_post_format());
    // Signal created OUTSIDE Suspense and seeded inside — the same shape as
    // ProfilePage's dn_field/bio_field, so the control's owner is the component,
    // not the transient Suspend scope.
    let format = RwSignal::new(PostFormat::Markdown);

    view! {
        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                format.set(initial.await.unwrap_or(PostFormat::Markdown));
                view! {
                    <label class="j-field-label" for="default-post-format">
                        "Default post format"
                    </label>
                    <select
                        id="default-post-format"
                        class="j-field-val"
                        on:change=move |ev| {
                            if let Ok(f) = event_target_value(&ev).parse::<PostFormat>() {
                                format.set(f);
                            }
                        }
                    >
                        {PostFormat::VARIANTS
                            .iter()
                            .copied()
                            .filter_map(|f| f.get_message().map(|label| (f, label)))
                            .map(|(f, label)| {
                                view! {
                                    <option value=f.to_string() selected=move || format.get() == f>
                                        {label}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                    <button
                        type="button"
                        class="j-btn"
                        on:click=move |_| {
                            action
                                .dispatch(SetDefaultPostFormat {
                                    format: format.get(),
                                });
                        }
                    >
                        "Save"
                    </button>
                }
            })}
        </Suspense>
    }
}

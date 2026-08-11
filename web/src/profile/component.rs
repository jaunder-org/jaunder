use crate::error::WebError;
use crate::forms::{Field, ValidatedInput, ValidatedTextarea};
use crate::topbar::Topbar;
use common::bio::Bio;
use common::display_name::DisplayName;
use common::render::PostFormat;
use leptos::prelude::*;

use super::api::{SetDefaultPostFormat, Update, get, get_default_post_format};

/// Profile page — shows username, display name, bio; allows updating.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<Update>::new();
    let profile = Resource::new(move || update_action.version().get(), |_| get());
    // Client-validated display name and bio (both optional: empty clears them),
    // owned by the component so the bespoke form can `.dispatch` the typed
    // `Update` args — the ADR-0065 direct-bind pattern (mirrors the post
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
                    <div class="j-card">
                        <div class="j-card-head">
                            <div>
                                <h2>"Default Post Format"</h2>
                                <div class="j-sub">"The editor format new posts start in."</div>
                            </div>
                        </div>
                        <div class="j-form-body">
                            <label class="j-form-field">
                                <span class="j-form-label">"Default post format"</span>
                                <select
                                    id="default-post-format"
                                    class="j-form-input"
                                    on:change=move |ev| {
                                        if let Ok(f) = event_target_value(&ev).parse::<PostFormat>()
                                        {
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
                                                <option
                                                    value=f.to_string()
                                                    selected=move || format.get() == f
                                                >
                                                    {label}
                                                </option>
                                            }
                                        })
                                        .collect_view()}
                                </select>
                            </label>
                        </div>
                        <div class="j-form-actions">
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
                        </div>
                    </div>
                }
            })}
        </Suspense>
    }
}

use super::{
    UpdateDefaultAudience, UpdateIdentity, UpdateIdentityRequest, UpdateMediaUploadsEnabled,
};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::reactive::Invalidator;
use crate::topbar::Topbar;
use crate::warning_revalidation::{SiteBaseUrlWarning, revalidates_warning};
use client::reactive;
use common::MutationOutcome;
use common::site::{SiteIdentity, SiteTagline, SiteTitle};
use common::tagged_url::BaseUrl;
use common::visibility::DefaultAudience;
use leptos::prelude::*;

#[component]
pub fn SiteSettingsPage() -> impl IntoView {
    let warning = expect_context::<SiteBaseUrlWarning>();
    let update_action = reactive::action_result_if(move || warning.notify(), revalidates_warning);
    // The same typed scope drives the form and its shell warning, so a settled
    // site-identity mutation re-reads both persisted projections together.
    let settings = reactive::resource(move || warning.track(), super::get_identity);

    view! {
        <Topbar title="Site Settings" sub="Operations" />
        <div class="j-scroll">
            <div class="j-settings">
                <Suspense fallback=|| {
                    view! { <p class="j-loading j-settings-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match settings.await {
                            Ok(identity) => site_settings_form(&identity, update_action).into_any(),
                            Err(error) => {
                                view! { <p class="error j-settings-error">{error.to_string()}</p> }
                                    .into_any()
                            }
                        }
                    })}
                </Suspense>
                <DefaultAudienceCard />
                <MediaUploadsCard />
                {move || {
                    update_action
                        .value()
                        .get()
                        .map(|result: Result<MutationOutcome<()>, WebError>| {
                            match crate::mutation_feedback::classify(
                                result,
                                "Save acknowledgement was lost; reload to verify the settings.",
                            ) {
                                crate::mutation_feedback::MutationFeedback::Confirmed(()) => {
                                    view! {
                                        <p class="success" role="status" data-settings-saved>
                                            "Site settings saved."
                                        </p>
                                    }
                                        .into_any()
                                }
                                crate::mutation_feedback::MutationFeedback::Error(message) => {
                                    view! { <p class="error j-settings-error">{message}</p> }
                                        .into_any()
                                }
                            }
                        })
                }}

            </div>
        </div>
    }
}

fn default_audience_label(audience: DefaultAudience) -> &'static str {
    match audience {
        DefaultAudience::Public => "Public",
        DefaultAudience::Subscribers => "Subscribers",
        DefaultAudience::Private => "Private",
    }
}

#[component]
fn DefaultAudienceCard() -> impl IntoView {
    use strum::VariantArray;

    let action = ServerAction::<UpdateDefaultAudience>::new();
    let initial = Resource::new(
        move || action.version().get(),
        |_| super::get_default_audience(),
    );
    let audience = RwSignal::new(None::<DefaultAudience>);
    let save = move |_| {
        if let Some(audience) = audience.get() {
            action.dispatch(UpdateDefaultAudience { audience });
        }
    };

    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading j-settings-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match initial.await {
                    Ok(current) => {
                        audience.set(Some(current));
                        view! {
                            <div class="j-card">
                                <div class="j-card-head">
                                    <div>
                                        <h2>"Site Default Audience"</h2>
                                        <div class="j-sub">
                                            "The fallback for new Posts when a User has no override."
                                        </div>
                                    </div>
                                </div>
                                <div class="j-form-body">
                                    <label class="j-form-field">
                                        <span class="j-form-label">"Site default audience"</span>
                                        <select
                                            id="site-default-audience"
                                            name="audience"
                                            class="j-form-input"
                                            prop:value=move || {
                                                audience
                                                    .get()
                                                    .map_or_else(String::new, |choice| choice.to_string())
                                            }
                                            on:change=move |event| {
                                                audience.set(event_target_value(&event).parse().ok());
                                            }
                                        >
                                            <For
                                                each=move || DefaultAudience::VARIANTS.iter().copied()
                                                key=|choice| *choice
                                                children=move |choice| {
                                                    view! {
                                                        <option value=choice
                                                            .to_string()>{default_audience_label(choice)}</option>
                                                    }
                                                }
                                            />
                                        </select>
                                    </label>
                                    <p class="j-sub">
                                        "Applies only when a new Post has neither an explicit Audience Selection nor a User Default Audience."
                                    </p>
                                </div>
                                <div class="j-form-actions">
                                    <button
                                        type="button"
                                        class="j-btn is-primary"
                                        prop:disabled=move || {
                                            audience.get().is_none() || action.pending().get()
                                        }
                                        on:click=save
                                    >
                                        "Save Site Default Audience"
                                    </button>
                                </div>
                            </div>
                        }
                            .into_any()
                    }
                    Err(error) => {
                        view! { <p class="error j-settings-error">{error.to_string()}</p> }
                            .into_any()
                    }
                }
            })}
        </Suspense>
        {move || {
            action
                .value()
                .get()
                .map(|result: Result<MutationOutcome<()>, WebError>| {
                    match crate::mutation_feedback::classify(
                        result,
                        "Save acknowledgement was lost; reload to verify the Site Default Audience.",
                    ) {
                        crate::mutation_feedback::MutationFeedback::Confirmed(()) => {
                            view! {
                                <p class="success" role="status" data-settings-saved>
                                    "Site Default Audience saved."
                                </p>
                            }
                                .into_any()
                        }
                        crate::mutation_feedback::MutationFeedback::Error(message) => {
                            view! { <p class="error j-settings-error">{message}</p> }.into_any()
                        }
                    }
                })
        }}
    }
}

#[component]
fn MediaUploadsCard() -> impl IntoView {
    // This card is a local scope: its action and persisted capability resource
    // share one bare Invalidator rather than a cross-component context newtype.
    let uploads = Invalidator::new();
    let update_action = reactive::action(move || uploads.notify());
    let uploads_enabled =
        reactive::resource(move || uploads.track(), super::get_media_uploads_enabled);

    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading j-settings-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match uploads_enabled.await {
                    Ok(enabled) => media_uploads_form(enabled, update_action).into_any(),
                    Err(error) => {
                        view! { <p class="error j-settings-error">{error.to_string()}</p> }
                            .into_any()
                    }
                }
            })}
        </Suspense>
        {move || {
            update_action
                .value()
                .get()
                .map(|result: Result<MutationOutcome<()>, WebError>| {
                    match crate::mutation_feedback::classify(
                        result,
                        "Save acknowledgement was lost; reload to verify media uploads.",
                    ) {
                        crate::mutation_feedback::MutationFeedback::Confirmed(()) => {
                            view! {
                                <p class="success" role="status" data-settings-saved>
                                    "Media upload settings saved."
                                </p>
                            }
                                .into_any()
                        }
                        crate::mutation_feedback::MutationFeedback::Error(message) => {
                            view! { <p class="error j-settings-error">{message}</p> }.into_any()
                        }
                    }
                })
        }}
    }
}

/// Renders the site-settings form, seeded from the persisted `identity`. The
/// component-owned fields are created **here** (inside the resolved-`identity`
/// scope, like the backup form) so the inputs render already populated. The save
/// button dispatches one typed `UpdateIdentityRequest` (ADR-0129): blank optional
/// tagline and base-URL fields yield `None`, clearing them on the wire.
fn site_settings_form(
    identity: &SiteIdentity,
    update_action: ServerAction<UpdateIdentity>,
) -> impl IntoView {
    let title_field = Field::<SiteTitle>::prefilled(&identity.title);
    let tagline_field =
        Field::<SiteTagline>::optional_prefilled(identity.tagline.as_deref().unwrap_or_default());
    let base_url_field =
        Field::<BaseUrl>::optional_prefilled(identity.base_url.as_deref().unwrap_or_default());
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        if update_action.pending().get()
            || !title_field.is_valid()
            || !tagline_field.is_valid()
            || !base_url_field.is_valid()
        {
            return;
        }
        if let Some(title) = title_field.parsed() {
            update_action.dispatch(UpdateIdentity {
                request: UpdateIdentityRequest {
                    title,
                    tagline: tagline_field.parsed(),
                    base_url: base_url_field.parsed(),
                },
            });
        }
    };
    view! {
        <form class="j-card" on:submit=submit>
            <div class="j-card-head">
                <div>
                    <h2>"Site Settings"</h2>
                    <div class="j-sub">
                        "Configure the Local title, optional tagline, and canonical base URL."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                <ValidatedInput<SiteTitle> label="Site title" name="title" field=title_field />
                <ValidatedInput<SiteTagline>
                    label="Site tagline"
                    name="tagline"
                    field=tagline_field
                    help="Leave blank to omit the Local description."
                />
                <ValidatedInput<BaseUrl>
                    label="Base URL"
                    name="base_url"
                    input_type="url"
                    field=base_url_field
                    help="Leave blank to disable or enter a fully-qualified https URL."
                />
            </div>
            <div class="j-form-actions">
                <button
                    type="submit"
                    class="j-btn is-primary"
                    prop:disabled=move || {
                        !title_field.is_valid() || !tagline_field.is_valid()
                            || !base_url_field.is_valid() || update_action.pending().get()
                    }
                >
                    "Save Site Settings"
                </button>
            </div>
        </form>
    }
}

/// Renders the independently persisted capability that allows new media uploads.
fn media_uploads_form(
    enabled: bool,
    update_action: ServerAction<UpdateMediaUploadsEnabled>,
) -> impl IntoView {
    let uploads_enabled = RwSignal::new(enabled);
    view! {
        <div class="j-card">
            <div class="j-card-head">
                <div>
                    <h2>"Media Uploads"</h2>
                    <div class="j-sub">
                        "Allow members to upload new media through the web interface and AtomPub."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                <label class="j-form-field j-form-toggle" for="media-uploads-enabled">
                    <input
                        id="media-uploads-enabled"
                        name="uploads_enabled"
                        type="checkbox"
                        prop:checked=move || uploads_enabled.get()
                        on:change=move |event| {
                            uploads_enabled.set(event_target_checked(&event));
                        }
                    />
                    <span class="j-form-label">"Enable new media uploads"</span>
                </label>
            </div>
            <div class="j-form-actions">
                <button
                    type="button"
                    class="j-btn is-primary"
                    on:click=move |_| {
                        update_action
                            .dispatch(UpdateMediaUploadsEnabled {
                                uploads_enabled: uploads_enabled.get(),
                            });
                    }
                >
                    "Save Media Uploads"
                </button>
            </div>
        </div>
    }
}

/// The #575 warning banner: shown in the authed admin chrome when `site.base_url` is
/// unconfigured (feeds/AtomPub disabled). A thin wrapper over the shared `WarnBanner`,
/// driven by the soft `is_base_url_warning_visible` server fn — hidden for non-operators
/// and once a base URL is set.
#[component]
pub fn SiteBaseUrlBanner() -> impl IntoView {
    let warning = expect_context::<SiteBaseUrlWarning>();
    let visible = reactive::resource(move || warning.track(), super::is_base_url_warning_visible);
    view! {
        <crate::banner::WarnBanner
            visible=visible
            message="Site base URL is not configured — feeds and AtomPub are disabled."
            links=vec![("/admin/site", "Site Settings")]
        />
    }
}

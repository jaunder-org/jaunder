use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::pages::Topbar;
use crate::site::{get_site_identity, UpdateSiteIdentity};
use common::absolute_url::AbsoluteUrl;
use common::site::{SiteIdentity, SiteTitle};
use leptos::prelude::*;

#[component]
pub fn SiteSettingsPage() -> impl IntoView {
    let update_action = ServerAction::<UpdateSiteIdentity>::new();
    let settings = Resource::new(
        move || update_action.version().get(),
        |_| get_site_identity(),
    );

    view! {
        <Topbar title="Site Settings" sub="Operations" />
        <div class="j-scroll">
            <div class="j-settings j-site-settings">
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
                {move || {
                    update_action
                        .value()
                        .get()
                        .map(|result: Result<(), WebError>| match result {
                            Ok(()) => {
                                view! {
                                    <p class="j-settings-saved" role="status">
                                        "Site settings saved."
                                    </p>
                                }
                                    .into_any()
                            }
                            Err(error) => {
                                view! { <p class="error j-settings-error">{error.to_string()}</p> }
                                    .into_any()
                            }
                        })
                }}
            </div>
        </div>
    }
}

// cov:ignore-start
/// Renders the site-settings form, seeded from the persisted `identity`. The
/// component-owned `title` buffer and the optional `base_url` `Field` are created
/// **here** (inside the resolved-`identity` scope, like the backup form) so the
/// inputs render already populated. The save button dispatches the typed
/// `UpdateSiteIdentity` args directly (ADR-0065): an empty base URL is valid, so
/// `parsed()` yields `None` and the field is omitted on the wire (clear-to-None).
fn site_settings_form(
    identity: &SiteIdentity,
    update_action: ServerAction<UpdateSiteIdentity>,
) -> impl IntoView {
    let title_field = Field::<SiteTitle>::prefilled(&identity.title);
    let base_url_field =
        Field::<AbsoluteUrl>::optional_prefilled(identity.base_url.as_deref().unwrap_or_default());
    let submit = move |_| {
        if let Some(title) = title_field.parsed() {
            update_action.dispatch(UpdateSiteIdentity {
                title,
                base_url: base_url_field.parsed(),
            });
        }
    };
    view! {
        <div class="j-card j-site-form">
            <div class="j-card-head">
                <div>
                    <h2>"Site Settings"</h2>
                    <div class="j-sub">"Configure the site title and canonical base URL."</div>
                </div>
            </div>
            <div class="j-site-form-body">
                <ValidatedInput<
                SiteTitle,
            >
                    label="Site Title"
                    name="title"
                    field=title_field
                    class="j-site-input"
                    field_class="j-site-field j-site-field-wide"
                />
                <ValidatedInput<
                AbsoluteUrl,
            >
                    label="Base URL"
                    name="base_url"
                    input_type="url"
                    field=base_url_field
                    field_class="j-site-field j-site-field-wide"
                    class="j-site-input"
                    help="Leave blank to disable or enter a fully-qualified https URL."
                />
            </div>
            <div class="j-site-form-actions">
                <button
                    type="button"
                    class="j-btn is-primary"
                    prop:disabled=move || !title_field.is_valid() || !base_url_field.is_valid()
                    on:click=submit
                >
                    "Save Site Settings"
                </button>
            </div>
        </div>
    }
}
// cov:ignore-stop

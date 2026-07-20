use crate::error::WebError;
use crate::pages::Topbar;
use crate::site::{get_site_identity, UpdateSiteIdentity};
use common::site::SiteIdentity;
use leptos::prelude::*;

#[component]
pub fn SiteSettingsPage() -> impl IntoView {
    let update_action = ServerAction::<UpdateSiteIdentity>::new();
    let settings = crate::server_resource(
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
                            Ok(identity) => site_settings_form(identity, update_action).into_any(),
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
fn site_settings_form(
    identity: SiteIdentity,
    update_action: ServerAction<UpdateSiteIdentity>,
) -> impl IntoView {
    view! {
        <ActionForm action=update_action attr:class="j-card j-site-form">
            <div class="j-card-head">
                <div>
                    <h2>"Site Settings"</h2>
                    <div class="j-sub">"Configure the site title and canonical base URL."</div>
                </div>
            </div>
            <div class="j-site-form-body">
                <label class="j-site-field j-site-field-wide">
                    <span class="j-edit-form-label">"Site Title"</span>
                    <input
                        class="j-site-input"
                        type="text"
                        name="title"
                        placeholder="My Site"
                        prop:value=identity.title
                    />
                </label>
                <label class="j-site-field j-site-field-wide">
                    <span class="j-edit-form-label">"Base URL"</span>
                    <input
                        class="j-site-input"
                        type="url"
                        name="base_url"
                        placeholder="https://example.com"
                        prop:value=identity.base_url.map(String::from).unwrap_or_default()
                    />
                    <span class="j-site-help">
                        "Leave blank to disable or enter a fully-qualified https URL."
                    </span>
                </label>
            </div>
            <div class="j-site-form-actions">
                <button type="submit" class="j-btn is-primary">
                    "Save Site Settings"
                </button>
            </div>
        </ActionForm>
    }
}
// cov:ignore-stop

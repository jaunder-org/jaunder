use crate::error::WebError;
use crate::pages::Topbar;
use crate::profile::{get_default_post_format, get_profile, SetDefaultPostFormat, UpdateProfile};
use leptos::prelude::*;

/// Profile page — shows username, display name, bio; allows updating.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<UpdateProfile>::new();
    let profile = crate::server_resource(move || update_action.version().get(), |_| get_profile());

    view! {
        <Topbar title="Profile".to_string() sub="Your details".to_string() />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match profile.await {
                            Ok(data) => {
                                view! {
                                    <p>"Username: " {data.username.to_string()}</p>
                                    <ActionForm action=update_action>
                                        <label>
                                            "Display Name"
                                            <input
                                                type="text"
                                                name="display_name"
                                                prop:value=data
                                                    .display_name
                                                    .clone()
                                                    .map(|d| d.to_string())
                                                    .unwrap_or_default()
                                            />
                                        </label>
                                        <label>
                                            "Bio"
                                            <textarea
                                                name="bio"
                                                prop:value=data.bio.clone().unwrap_or_default()
                                            />
                                        </label>
                                        <button type="submit" class="j-btn is-primary">
                                            "Update Profile"
                                        </button>
                                    </ActionForm>
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
#[component]
fn DefaultPostFormatControl() -> impl IntoView {
    let action = ServerAction::<SetDefaultPostFormat>::new();
    let initial = crate::server_resource(|| (), |()| get_default_post_format());

    view! {
        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                let current = initial.await.unwrap_or_else(|_| "html".to_string());
                view! {
                    <ActionForm action=action>
                        <label class="j-field-label">"Default post format"</label>
                        <select class="j-field-val" name="format">
                            <option value="markdown" selected=current == "markdown">
                                "Markdown"
                            </option>
                            <option value="org" selected=current == "org">
                                "Org"
                            </option>
                            <option value="html" selected=current == "html">
                                "HTML"
                            </option>
                        </select>
                        <button type="submit">"Save"</button>
                    </ActionForm>
                }
            })}
        </Suspense>
    }
}

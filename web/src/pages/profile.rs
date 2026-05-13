use crate::error::WebError;
use crate::pages::Topbar;
use crate::profile::{get_profile, UpdateProfile};
use leptos::prelude::*;

/// Profile page — shows username, display name, bio; allows updating.
#[allow(clippy::must_use_candidate)]
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<UpdateProfile>::new();
    let profile = Resource::new(move || update_action.version().get(), |_| get_profile());

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
                                    <p>"Username: " {data.username.clone()}</p>
                                    <ActionForm action=update_action>
                                        <label>
                                            "Display Name"
                                            <input
                                                type="text"
                                                name="display_name"
                                                prop:value=data.display_name.clone().unwrap_or_default()
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

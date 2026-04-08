use crate::auth::get_registration_policy;
use crate::invites::{list_invites, CreateInvite};
use leptos::prelude::*;

/// Invites page — lists invite codes; allows creating new ones.
/// Returns 404 (via SSR response options) when the registration policy is not
/// `invite_only`.
#[component]
pub fn InvitesPage() -> impl IntoView {
    let create_action = ServerAction::<CreateInvite>::new();
    let policy = Resource::new(|| (), |_| get_registration_policy());
    let invites = Resource::new(move || create_action.version().get(), |_| list_invites());

    view! {
        <h1>"Invites"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                let policy_str = policy.await.unwrap_or_default();
                if policy_str != "invite_only" {
                    #[cfg(feature = "ssr")]
                    {
                        use leptos::context::use_context;
                        use leptos_axum::ResponseOptions;
                        if let Some(opts) = use_context::<ResponseOptions>() {
                            opts.set_status(axum::http::StatusCode::NOT_FOUND);
                        }
                    }
                    return // Set 404 status when rendered server-side.
                    view! { <p>"Page not found."</p> }
                        .into_any();
                }
                match invites.await {
                    Ok(list) => {
                        view! {
                            <ActionForm action=create_action>
                                <label>
                                    "Expires in hours"
                                    <input type="number" name="expires_in_hours" />
                                </label>
                                <button type="submit">"Create Invite"</button>
                            </ActionForm>
                            <ul>
                                {list
                                    .into_iter()
                                    .map(|i| {
                                        view! {
                                            <li>
                                                "Code: " {i.code.clone()} " — expires: "
                                                {i.expires_at.clone()}
                                                {i
                                                    .used_at
                                                    .clone()
                                                    .map(|t| {
                                                        view! {
                                                            " (used at "
                                                            {t}
                                                            ")"
                                                        }
                                                    })}
                                            </li>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </ul>
                        }
                            .into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

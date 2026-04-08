use crate::email::{verify_email, RequestEmailVerification};
use crate::profile::get_profile;
use leptos::prelude::*;

/// Email settings page — shows current email and verification status;
/// form to submit a new email address for verification.
#[component]
pub fn EmailPage() -> impl IntoView {
    let request_action = ServerAction::<RequestEmailVerification>::new();
    let profile = Resource::new(move || request_action.version().get(), |_| get_profile());

    view! {
        <h1>"Email Settings"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match profile.await {
                    Ok(data) => {
                        let email_status = match (data.email.clone(), data.email_verified) {
                            (Some(ref e), true) => format!("{e} (verified)"),
                            (Some(ref e), false) => format!("{e} (unverified)"),
                            (None, _) => "No email set".to_string(),
                        };
                        view! {
                            <p>"Current email: " {email_status}</p>
                            <ActionForm action=request_action>
                                <label>
                                    "New email address" <input type="email" name="email" />
                                </label>
                                <button type="submit">"Send verification link"</button>
                            </ActionForm>
                        }
                            .into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            request_action
                .value()
                .get()
                .map(|r: Result<(), ServerFnError>| match r {
                    Ok(()) => {
                        view! { <p>"Check your email for a verification link."</p> }.into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                })
        }}
    }
}

/// Reads the `token` query parameter and calls `verify_email` on mount.
/// Renders a success message or an appropriate error.
#[component]
pub fn VerifyEmailPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    let query = use_query_map();
    let token = move || query.with(|q| q.get("token").unwrap_or_default());
    let result = Resource::new(token, verify_email);

    view! {
        <h1>"Verify Email"</h1>
        <Suspense fallback=|| {
            view! { <p>"Verifying..."</p> }
        }>
            {move || Suspend::new(async move {
                match result.await {
                    Ok(()) => view! { <p>"Your email address has been verified."</p> }.into_any(),
                    Err(e) => {
                        let msg = e.to_string();
                        view! { <p class="error">{msg}</p> }.into_any()
                    }
                }
            })}
        </Suspense>
    }
}

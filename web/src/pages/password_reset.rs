use crate::password_reset::{ConfirmPasswordReset, RequestPasswordReset};
use leptos::prelude::*;
use leptos_router::components::Redirect;

/// Username form for requesting a password reset.
///
/// On success renders a neutral confirmation message. On error (no verified
/// email / contact operator) surfaces the error message directly.
#[component]
pub fn ForgotPasswordPage() -> impl IntoView {
    let request_action = ServerAction::<RequestPasswordReset>::new();

    view! {
        <h1>"Forgot Password"</h1>
        <ActionForm action=request_action>
            <label>"Username" <input type="text" name="username" /></label>
            <button type="submit">"Send reset link"</button>
        </ActionForm>
        {move || {
            request_action
                .value()
                .get()
                .map(|r: Result<(), ServerFnError>| match r {
                    Ok(()) => {
                        view! {
                            <p>
                                "If there is a verified email address on file, a reset link has been sent. Check your email."
                            </p>
                        }
                            .into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                })
        }}
    }
}

/// Reads the `token` query parameter; shows a new-password form.
/// On success redirects to `/login`.
#[component]
pub fn ResetPasswordPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    let query = use_query_map();
    let token = move || query.with(|q| q.get("token").unwrap_or_default());

    let confirm_action = ServerAction::<ConfirmPasswordReset>::new();

    view! {
        <h1>"Reset Password"</h1>
        <ActionForm action=confirm_action>
            <input type="hidden" name="token" prop:value=token />
            <label>"New password" <input type="password" name="new_password" /></label>
            <button type="submit">"Set new password"</button>
        </ActionForm>
        {move || {
            confirm_action
                .value()
                .get()
                .map(|r: Result<(), ServerFnError>| match r {
                    Ok(()) => view! { <Redirect path="/login" /> }.into_any(),
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                })
        }}
    }
}

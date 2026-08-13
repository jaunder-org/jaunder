//! Password-reset vertical — wasm-only UI (ADR-0070): the forgot-password and
//! reset-password pages.

use super::{Confirm, ConfirmPasswordResetRequest, Request};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput, request_submit_gate};
use crate::topbar::Topbar;
use common::password::ProfferedPassword;
use common::token::RawToken;
use common::username::Username;
use leptos::prelude::*;
use leptos_router::components::Redirect;

/// Username form for requesting a password reset.
///
/// On success renders a neutral confirmation message. On error (no verified
/// email / contact operator) surfaces the error message directly.
#[component]
pub fn ForgotPasswordPage() -> impl IntoView {
    let request_action = ServerAction::<Request>::new();
    let username = Field::<Username>::new();

    view! {
        <Topbar title="Forgot Password" sub="Recover access" />
        <div class="j-scroll">
            <div class="j-page">
                <ActionForm action=request_action>
                    <ValidatedInput<Username>
                        label="Username"
                        name="username"
                        autocomplete="username"
                        field=username
                        transform=str::to_lowercase
                    />
                    <button
                        type="submit"
                        class="j-btn is-primary"
                        prop:disabled=move || !username.is_valid()
                    >
                        "Send reset link"
                    </button>
                </ActionForm>
                {move || {
                    request_action
                        .value()
                        .get()
                        .map(|r: Result<(), WebError>| match r {
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
            </div>
        </div>
    }
}

/// Reads the `token` query parameter; shows a new-password form.
/// On success redirects to `/login`.
#[component]
pub fn ResetPasswordPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    // The reset token arrives in the URL (`?token=…`) from the emailed reset link,
    // not typed by hand. Read and parse it once at mount — a plain read is safe here
    // because the app is CSR (no SSR-hydration race; see the spec/#433).
    let token = use_query_map()
        .read()
        .get("token")
        .and_then(|value| value.parse::<RawToken>().ok());
    let confirm_action = ServerAction::<Confirm>::new();
    let new_password = Field::<ProfferedPassword>::new();
    let (disabled, dispatch) = request_submit_gate(
        confirm_action.pending().into(),
        Callback::new(move |()| {
            token
                .clone()
                .zip(new_password.parsed())
                .map(|(token, new_password)| ConfirmPasswordResetRequest {
                    token,
                    new_password,
                })
        }),
        Callback::new(move |request| {
            confirm_action.dispatch(Confirm { request });
        }),
    );
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        dispatch.run(());
    };

    view! {
        <Topbar title="Reset Password" sub="Set a new password" />
        <div class="j-scroll">
            <div class="j-page">
                <form on:submit=submit>
                    <ValidatedInput<ProfferedPassword>
                        label="New password"
                        name="new_password"
                        input_type="password"
                        autocomplete="new-password"
                        field=new_password
                    />
                    <button
                        type="submit"
                        class="j-btn is-primary"
                        prop:disabled=move || disabled.get()
                    >
                        "Set new password"
                    </button>
                </form>
                {move || {
                    confirm_action
                        .value()
                        .get()
                        .map(|r: Result<(), WebError>| match r {
                            Ok(()) => view! { <Redirect path="/login" /> }.into_any(),
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        })
                }}
            </div>
        </div>
    }
}

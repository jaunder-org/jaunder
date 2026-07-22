//! Email vertical — wasm-only UI (ADR-0070): the email-settings page and the
//! verify-email landing.

use super::{email_status_line, parse_verification_token, verify_email, RequestEmailVerification};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::profile::get_profile;
use crate::topbar::Topbar;
use common::email::Email;
use leptos::prelude::*;

/// Email settings page — shows current email and verification status;
/// form to submit a new email address for verification.
#[component]
pub fn EmailPage() -> impl IntoView {
    let request_action = ServerAction::<RequestEmailVerification>::new();
    let email = Field::<Email>::new();
    let profile = Resource::new(move || request_action.version().get(), |_| get_profile());

    view! {
        <Topbar title="Email" sub="Verify your address" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match profile.await {
                            Ok(data) => {
                                let email_status = email_status_line(
                                    data.email.as_ref(),
                                    data.email_verified,
                                );
                                view! { <p>"Current email: " {email_status}</p> }.into_any()
                            }
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
                <ActionForm action=request_action>
                    <ValidatedInput<
                    Email,
                >
                        label="New email address"
                        name="email"
                        input_type="email"
                        autocomplete="email"
                        field=email
                    />
                    <button
                        type="submit"
                        class="j-btn is-primary"
                        prop:disabled=move || !email.is_valid()
                    >
                        "Send verification link"
                    </button>
                </ActionForm>
                {move || {
                    request_action
                        .value()
                        .get()
                        .map(|r: Result<(), WebError>| match r {
                            Ok(()) => {
                                view! { <p>"Check your email for a verification link."</p> }
                                    .into_any()
                            }
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        })
                }}
            </div>
        </div>
    }
}

/// Reads the `token` query parameter and calls `verify_email` on mount.
/// Renders a success message or an appropriate error.
#[component]
pub fn VerifyEmailPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    let query = use_query_map();
    let token = move || query.with(|q| q.get("token").unwrap_or_default());
    // `verify_email` takes a typed `RawToken`. Parse the URL's token client-side
    // (ADR-0065 pre-validation): a malformed token short-circuits to a validation error
    // with no server round-trip; a well-formed one is verified server-side as before.
    let result = Resource::new(token, |raw: String| async move {
        let token = parse_verification_token(&raw)?;
        verify_email(token).await
    });

    view! {
        <Topbar title="Verify Email" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Verifying\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match result.await {
                            Ok(()) => {
                                view! { <p>"Your email address has been verified."</p> }.into_any()
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                view! { <p class="error">{msg}</p> }.into_any()
                            }
                        }
                    })}
                </Suspense>
            </div>
        </div>
    }
}

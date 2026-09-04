//! The **registration** vertical's wasm-only UI (ADR-0070): `RegisterPage` and the
//! invite-guidance view. Never host-compiled — calls browser primitives (the auth
//! [`marker_storage`](crate::auth::marker_storage) binding) directly, no `cfg`
//! gates inside this file.

use super::Register;
use crate::auth::{self, SessionUser};
use crate::error::WebError;
use crate::forms::{self, Field, ValidatedInput};
use crate::topbar::Topbar;
use common::{
    MutationOutcome, password::PasswordShape, registration::RegistrationPolicy, username::Username,
};
use leptos::prelude::*;

/// Guidance shown on `/register` under either invitation policy when the URL carries no invite
/// code: the visitor did not follow an invitation link, so a register form would only fail with
/// "invite code required." Exercised by the invite e2e (Test B), not host tests.
#[component]
fn InviteLinkRequired() -> impl IntoView {
    view! {
        <div class="j-card">
            <p class="j-form-note">
                "You need an invitation link to register. Please use the link from your invitation email."
            </p>
        </div>
    }
}

/// Registration page.
#[component]
pub fn RegisterPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    let register_action = ServerAction::<Register>::new();
    let policy = Resource::new(|| (), |()| super::get_policy());

    // The invite code arrives in the URL (`?invite_code=…`) from the invitation link,
    // not typed by hand. Read it once at mount — a plain read is safe here because the
    // app is CSR (no SSR-hydration race; see the spec/#433).
    let invite_code = use_query_map()
        .read()
        .get("invite_code")
        .unwrap_or_default();

    // On a successful register, set the shared session (#591): a new user is never
    // an operator (`is_operator: false`); this updates the reactive signal (chrome
    // flips without a reload) and the advisory marker (#181, ADR-0044) for the next
    // pre-paint boot. Read the *submitted* username from the action input, not the
    // live `username` field, which the user could have edited between submit and
    // response. The server still owns the real cookie.
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = register_action.value().get()
            && let Some(input) = register_action.input().get()
        {
            match outcome {
                MutationOutcome::Confirmed(()) | MutationOutcome::CommitIndeterminate(()) => {
                    auth::set_session(SessionUser {
                        username: input.username.clone(),
                        is_operator: false,
                    });
                    auth::use_session().reconcile.refetch();
                }
            }
        }
    });

    view! {
        <Topbar title="Register".to_string() sub="Create your account".to_string() />
        <div class="j-scroll">
            <div class="j-page-narrow">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || {
                        let invite_code = invite_code.clone();
                        Suspend::new(async move {
                            let p = policy.await;
                            let requires_invitation = p
                                .is_ok_and(RegistrationPolicy::requires_invitation);
                            if requires_invitation && invite_code.is_empty() {
                                return view! { <InviteLinkRequired /> }.into_any();
                            }
                            // No code in the URL under an invitation policy: guide, don't show
                            // a form that would only fail server-side.

                            view! {
                                <RegistrationForm
                                    action=register_action
                                    invite_code=invite_code.clone()
                                    show_invite_note=requires_invitation
                                />
                            }
                                .into_any()
                        })
                    }}
                </Suspense>
                {move || {
                    register_action
                        .value()
                        .get()
                        .and_then(|r: Result<MutationOutcome<()>, WebError>| match r {
                            Ok(MutationOutcome::Confirmed(())) => None,
                            Ok(MutationOutcome::CommitIndeterminate(())) => {
                                Some(
                                    view! {
                                        <p class="error">
                                            "Your account may have been created, but its status could not be confirmed. Refresh to check."
                                        </p>
                                    }
                                        .into_any(),
                                )
                            }
                            Err(error) => {
                                Some(view! { <p class="error">{error.to_string()}</p> }.into_any())
                            }
                        })
                }}
            </div>
        </div>
    }
}

#[component]
fn RegistrationForm(
    action: ServerAction<Register>,
    invite_code: String,
    show_invite_note: bool,
) -> impl IntoView {
    let username = Field::<Username>::new();
    let password = Field::<PasswordShape>::new();
    let (disabled, submit) = forms::server_action_submit(action, move || {
        username
            .parsed()
            .zip(password.value().parse().ok())
            .map(|(username, password)| Register {
                username,
                password,
                invite_code: (!invite_code.is_empty())
                    .then(|| invite_code.parse().ok())
                    .flatten(),
            })
    });

    view! {
        <form class="j-card" on:submit=submit>
            <div class="j-card-head">
                <h2>"Create an account"</h2>
            </div>
            <div class="j-form-body">
                <ValidatedInput<Username>
                    label="Username"
                    name="username"
                    autocomplete="username"
                    field=username
                    transform=str::to_lowercase
                />
                <ValidatedInput<PasswordShape>
                    label="Password"
                    name="password"
                    input_type="password"
                    autocomplete="new-password"
                    field=password
                />
                {show_invite_note
                    .then(|| {
                        view! { <p class="j-form-note">"Registering with your invitation."</p> }
                    })}
            </div>
            <div class="j-form-actions">
                <button type="submit" class="j-btn is-primary" prop:disabled=move || disabled.get()>
                    "Register"
                </button>
            </div>
        </form>
    }
}

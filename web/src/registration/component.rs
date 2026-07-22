//! The **registration** vertical's wasm-only UI (ADR-0070): `RegisterPage` and the
//! invite-guidance view. Never host-compiled — calls browser primitives (the auth
//! [`marker_storage`](crate::auth::marker_storage) binding) directly, no `cfg`
//! gates inside this file.

use super::{get_registration_policy, Register};
use crate::auth::{set_session, SessionUser};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::topbar::Topbar;
use common::password::Password;
use common::registration::RegistrationPolicy;
use common::token::RawToken;
use common::username::Username;
use leptos::prelude::*;

/// Guidance shown on `/register` in invite-only mode when the URL carries no invite
/// code — the visitor didn't follow an invitation link, so a register form would only
/// fail "invite code required". (#444 will turn this into a request-an-invitation form.)
/// Exercised by the invite e2e (Test B), not host tests.
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
    let policy = Resource::new(|| (), |()| get_registration_policy());
    let username = Field::<Username>::new();
    let password = Field::<Password>::new();

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
        if let Some(Ok(_)) = register_action.value().get() {
            if let Some(input) = register_action.input().get() {
                set_session(SessionUser {
                    username: input.username,
                    is_operator: false,
                });
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
                            let is_invite_only = matches!(p, Ok(RegistrationPolicy::InviteOnly));
                            if is_invite_only && invite_code.is_empty() {
                                return view! { <InviteLinkRequired /> }.into_any();
                            }
                            // No code in the URL under invite-only: guide, don't show a
                            // form that would only fail server-side.

                            view! {
                                <ActionForm action=register_action attr:class="j-card">
                                    <div class="j-card-head">
                                        <h2>"Create an account"</h2>
                                    </div>
                                    <div class="j-form-body">
                                        <ValidatedInput<
                                        Username,
                                    >
                                            label="Username"
                                            name="username"
                                            autocomplete="username"
                                            field=username
                                            transform=str::to_lowercase
                                        />
                                        <ValidatedInput<
                                        Password,
                                    >
                                            label="Password"
                                            name="password"
                                            input_type="password"
                                            autocomplete="new-password"
                                            field=password
                                        />
                                        {(is_invite_only && !invite_code.is_empty())
                                            .then(|| {
                                                // The code comes from the invitation link — carried as a
                                                // hidden field and confirmed read-only, never typed.
                                                view! {
                                                    <input
                                                        type="hidden"
                                                        name="invite_code"
                                                        value=invite_code.clone()
                                                    />
                                                    <p class="j-form-note">
                                                        "Registering with your invitation."
                                                    </p>
                                                }
                                            })}
                                    </div>
                                    <div class="j-form-actions">
                                        <button
                                            type="submit"
                                            class="j-btn is-primary"
                                            prop:disabled=move || {
                                                !(username.is_valid() && password.is_valid())
                                            }
                                        >
                                            "Register"
                                        </button>
                                    </div>
                                </ActionForm>
                            }
                                .into_any()
                        })
                    }}
                </Suspense>
                {move || {
                    register_action
                        .value()
                        .get()
                        .and_then(|r: Result<RawToken, WebError>| r.err())
                        .map(|e| view! { <p class="error">{e.to_string()}</p> })
                }}
            </div>
        </div>
    }
}

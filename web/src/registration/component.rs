//! The **registration** vertical's wasm-only UI (ADR-0070): `RegisterPage` and the
//! invite-guidance view. Never host-compiled — calls browser primitives (the auth
//! [`marker_storage`](crate::auth::marker_storage) binding) directly, no `cfg`
//! gates inside this file.

use super::{Register, RegistrationRequest, get_policy};
use crate::auth::{SessionUser, set_session};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput, pair_submit_gate};
use crate::topbar::Topbar;
use common::invite::ProfferedInviteCode;
use common::password::ProfferedPassword;
use common::registration::RegistrationPolicy;
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
    let policy = Resource::new(|| (), |()| get_policy());

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
        if let Some(Ok(())) = register_action.value().get()
            && let Some(input) = register_action.input().get()
        {
            set_session(SessionUser {
                username: input.request.username.clone(),
                is_operator: false,
            });
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
                                <RegistrationForm
                                    action=register_action
                                    invite_code=invite_code.clone()
                                    show_invite_note=is_invite_only
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
                        .and_then(|r: Result<(), WebError>| r.err())
                        .map(|e| view! { <p class="error">{e.to_string()}</p> })
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
    let password = Field::<ProfferedPassword>::new();
    let parsed_invite = (!invite_code.is_empty())
        .then(|| invite_code.parse::<ProfferedInviteCode>().ok())
        .flatten();
    let (disabled, dispatch) = pair_submit_gate(
        username,
        password,
        action.pending().into(),
        Callback::new(move |(username, password)| {
            action.dispatch(Register {
                request: RegistrationRequest {
                    username,
                    password,
                    invite_code: parsed_invite.clone(),
                },
            });
        }),
    );
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        dispatch.run(());
    };

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
                <ValidatedInput<ProfferedPassword>
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

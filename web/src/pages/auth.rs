use crate::auth::{get_registration_policy, Login, Logout, Register};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::pages::Topbar;
use common::password::Password;
use common::username::Username;
use leptos::prelude::*;
use macros::client_only;

/// Guidance shown on `/register` in invite-only mode when the URL carries no invite
/// code — the visitor didn't follow an invitation link, so a register form would only
/// fail "invite code required". (#444 will turn this into a request-an-invitation form.)
/// Client-only UI, exercised by the invite e2e (Test B), not host tests.
#[client_only]
fn invite_link_required() -> AnyView {
    view! {
        <div class="j-card">
            <p class="j-form-note">
                "You need an invitation link to register. Please use the link from your invitation email."
            </p>
        </div>
    }
    .into_any()
}

/// Registration page.
#[component]
pub fn RegisterPage() -> impl IntoView {
    use leptos_router::hooks::use_query_map;

    let register_action = ServerAction::<Register>::new();
    let policy = crate::server_resource(|| (), |()| get_registration_policy());
    let username = Field::<Username>::new();
    let password = Field::<Password>::new();

    // The invite code arrives in the URL (`?invite_code=…`) from the invitation link,
    // not typed by hand. Read it once at mount — a plain read is safe here because the
    // app is CSR (no SSR-hydration race; see the spec/#433).
    let invite_code = use_query_map()
        .read()
        .get("invite_code")
        .unwrap_or_default();

    // Mirror the new session into the advisory auth marker (#181, ADR-0044): on a
    // successful register the client knows the submitted username, so pre-paint
    // auth works on the very next navigation. wasm-only (localStorage); the server
    // still owns the real session cookie.
    Effect::new(move |_| {
        if let Some(Ok(_)) = register_action.value().get() {
            crate::auth::marker_storage::set(&username.value.get_untracked());
        }
    });

    view! {
        <Topbar title="Register" sub="Create your account" />
        <div class="j-scroll">
            <div class="j-page-narrow">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || {
                        let invite_code = invite_code.clone();
                        Suspend::new(async move {
                            let p = policy.await;
                            let is_invite_only = p.as_deref() == Ok("invite_only");
                            if is_invite_only && invite_code.is_empty() {
                                return invite_link_required();
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
                        .and_then(|r: Result<String, WebError>| r.err())
                        .map(|e| view! { <p class="error">{e.to_string()}</p> })
                }}
            </div>
        </div>
    }
}

/// Login page.
#[component]
pub fn LoginPage() -> impl IntoView {
    let login_action = ServerAction::<Login>::new();
    let username = Field::<Username>::new();
    let password = Field::<Password>::new();

    // Mirror the session into the advisory auth marker on a successful login
    // (#181, ADR-0044) — the client's synchronous pre-paint boot source. wasm-only.
    Effect::new(move |_| {
        if let Some(Ok(_)) = login_action.value().get() {
            crate::auth::marker_storage::set(&username.value.get_untracked());
        }
    });

    view! {
        <Topbar title="Login" sub="Sign in to your account" />
        <div class="j-scroll">
            <div class="j-page-narrow">
                <ActionForm action=login_action attr:class="j-card">
                    <div class="j-card-head">
                        <h2>"Sign in"</h2>
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
                            autocomplete="current-password"
                            field=password
                        />
                    </div>
                    <div class="j-form-actions">
                        <button
                            type="submit"
                            class="j-btn is-primary"
                            prop:disabled=move || !(username.is_valid() && password.is_valid())
                        >
                            "Login"
                        </button>
                    </div>
                </ActionForm>
                {move || {
                    login_action
                        .value()
                        .get()
                        .map(|r: Result<String, WebError>| match r {
                            Ok(_) => {
                                view! { <p class="j-loading">"Logging in\u{2026}"</p> }.into_any()
                            }
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        })
                }}
            </div>
        </div>
    }
}

/// Logout page — fires the logout server action on mount.
#[component]
pub fn LogoutPage() -> impl IntoView {
    let logout_action = ServerAction::<Logout>::new();

    Effect::new(move |_| {
        logout_action.dispatch(Logout {});
    });

    // Clear the advisory auth marker once logout succeeds (#181, ADR-0044) so the
    // next paint is anonymous. wasm-only; the server clears the real cookie.
    Effect::new(move |_| {
        if let Some(Ok(())) = logout_action.value().get() {
            crate::auth::marker_storage::clear();
        }
    });

    view! {
        <Topbar title="Logout" />
        <div class="j-scroll">
            <div class="j-page">
                <p class="j-loading">"Logging out\u{2026}"</p>
                {move || {
                    logout_action
                        .value()
                        .get()
                        .map(|r: Result<(), WebError>| {
                            match r {
                                Ok(()) => view! { <p>"You have been logged out."</p> }.into_any(),
                                Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                            }
                        })
                }}
            </div>
        </div>
    }
}

use crate::auth::{get_registration_policy, Login, Logout, Register};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::pages::Topbar;
use common::password::Password;
use common::username::Username;
use leptos::prelude::*;

/// Registration page.
#[component]
pub fn RegisterPage() -> impl IntoView {
    let register_action = ServerAction::<Register>::new();
    let policy = crate::server_resource(|| (), |()| get_registration_policy());
    let username = Field::<Username>::new();
    let password = Field::<Password>::new();

    // Mirror the new session into the advisory auth marker (#181, ADR-0044): on a
    // successful register the client knows the submitted username, so pre-paint
    // auth works on the very next navigation. wasm-only (localStorage); the server
    // still owns the real session cookie.
    Effect::new(move |_| {
        if let Some(Ok(_)) = register_action.value().get() {
            crate::auth::marker::set(&username.value.get_untracked());
        }
    });

    view! {
        <Topbar title="Register".to_string() sub="Create your account".to_string() />
        <div class="j-scroll">
            <div class="j-page-narrow">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        let p = policy.await;
                        let is_invite_only = p.as_deref() == Ok("invite_only");
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

                                    {is_invite_only
                                        .then(|| {
                                            view! {
                                                <label class="j-form-field">
                                                    <span class="j-form-label">"Invite code"</span>
                                                    // `required`: the field renders only in
                                                    // invite-only mode, where a code is
                                                    // mandatory — this blocks an empty submit
                                                    // client-side so the typed wire arg
                                                    // (`Option<ProfferedInviteCode>`) only ever
                                                    // sees `None` (open mode) or a filled value.
                                                    <input
                                                        class="j-form-input"
                                                        type="text"
                                                        name="invite_code"
                                                        required=true
                                                    />
                                                </label>
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
                    })}
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
            crate::auth::marker::set(&username.value.get_untracked());
        }
    });

    view! {
        <Topbar title="Login".to_string() sub="Sign in to your account".to_string() />
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
            crate::auth::marker::clear();
        }
    });

    view! {
        <Topbar title="Logout".to_string() />
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

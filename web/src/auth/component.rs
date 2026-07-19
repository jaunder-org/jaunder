//! The **auth** vertical's wasm-only UI (ADR-0070): `LoginPage` and `LogoutPage`.
//! Never host-compiled — free to call browser primitives (the advisory auth
//! [`marker_storage`](super::marker_storage) binding) directly, no `cfg` gates
//! inside this file.

use super::{marker_storage, Login, Logout};
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::topbar::Topbar;
use common::password::Password;
use common::username::Username;
use leptos::prelude::*;

/// Login page.
#[component]
pub fn LoginPage() -> impl IntoView {
    let login_action = ServerAction::<Login>::new();
    let username = Field::<Username>::new();
    let password = Field::<Password>::new();

    // Mirror the session into the advisory auth marker on a successful login
    // (#181, ADR-0044) — the client's synchronous pre-paint boot source. Read the
    // *submitted* username from the action input, not the live `username` field,
    // which the user could have edited between submit and response.
    Effect::new(move |_| {
        if let Some(Ok(_)) = login_action.value().get() {
            if let Some(input) = login_action.input().get() {
                marker_storage::set(input.username.as_ref());
            }
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
    // next paint is anonymous. The server clears the real cookie.
    Effect::new(move |_| {
        if let Some(Ok(())) = logout_action.value().get() {
            marker_storage::remove();
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

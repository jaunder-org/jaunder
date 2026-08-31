//! The **auth** vertical's wasm-only UI (ADR-0070): `LoginPage` and `LogoutPage`.
//! Never host-compiled — free to call browser primitives (the advisory auth
//! [`marker_storage`](super::marker_storage) binding) directly, no `cfg` gates
//! inside this file.

use super::{Login, Logout, SessionUser};
use crate::error::WebError;
use crate::forms::{self, Field, ValidatedInput};
use crate::topbar::Topbar;
use common::{MutationOutcome, password::PasswordShape, username::Username};
use leptos::prelude::*;

/// Login page.
#[component]
pub fn LoginPage() -> impl IntoView {
    let login_action = ServerAction::<Login>::new();

    // On a successful login, store the returned session directly: this updates the
    // reactive signal so the chrome flips without a document reload, and mirrors it
    // into the advisory marker (#181, ADR-0044) for the next pre-paint boot.
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = login_action.value().get() {
            match outcome {
                MutationOutcome::Confirmed(session)
                | MutationOutcome::CommitIndeterminate(session) => {
                    super::set_session(session);
                    super::use_session().reconcile.refetch();
                }
            }
        }
    });

    view! {
        <Topbar title="Login".to_string() sub="Sign in to your account".to_string() />
        <div class="j-scroll">
            <div class="j-page-narrow">
                <LoginForm action=login_action />
                {move || {
                    login_action
                        .value()
                        .get()
                        .map(|r: Result<MutationOutcome<SessionUser>, WebError>| match r {
                            Ok(MutationOutcome::Confirmed(_)) => {
                                view! { <p class="j-loading">"Logging in\u{2026}"</p> }.into_any()
                            }
                            Ok(MutationOutcome::CommitIndeterminate(_)) => {
                                view! {
                                    <p class="error">
                                        "Your sign-in may have succeeded, but its status could not be confirmed. Refresh to check."
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

/// Native login form: validates typed domain values before dispatching the
/// generated flat server-function input.
#[component]
fn LoginForm(action: ServerAction<Login>) -> impl IntoView {
    let username = Field::<Username>::new();
    let password = Field::<PasswordShape>::new();
    let (disabled, submit) = forms::server_action_submit(action, move || {
        username
            .parsed()
            .zip(password.value.get().parse().ok())
            .map(|(username, password)| Login {
                username,
                password,
                label: None,
            })
    });

    view! {
        <form class="j-card" on:submit=submit>
            <div class="j-card-head">
                <h2>"Sign in"</h2>
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
                    autocomplete="current-password"
                    field=password
                />
            </div>
            <div class="j-form-actions">
                <button type="submit" class="j-btn is-primary" prop:disabled=move || disabled.get()>
                    "Login"
                </button>
            </div>
        </form>
    }
}

/// Logout page — fires the logout server action on mount.
#[component]
pub fn LogoutPage() -> impl IntoView {
    let logout_action = ServerAction::<Logout>::new();

    Effect::new(move |_| {
        logout_action.dispatch(Logout {});
    });

    // On logout, clear the shared session (#591): resets the reactive signal (chrome
    // goes anonymous without a reload) and removes the advisory marker (#181,
    // ADR-0044). The server clears the real cookie.
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = logout_action.value().get() {
            match outcome {
                MutationOutcome::Confirmed(()) | MutationOutcome::CommitIndeterminate(()) => {
                    super::clear_session();
                    super::use_session().reconcile.refetch();
                }
            }
        }
    });

    // What actually paints: a "Logging out…" transient during the round-trip, then on
    // success leptos_router's redirect->pushState navigates to "/" (no full reload,
    // #591) on the same resolution that fills the action value — so a logout
    // *failure* (no redirect) is the only case the resolution block below can show (#649).
    view! {
        <Topbar title="Logout".to_string() />
        <div class="j-scroll">
            <div class="j-page">
                <p class="j-loading">"Logging out\u{2026}"</p>
                {move || {
                    logout_action
                        .value()
                        .get()
                        .and_then(|result: Result<MutationOutcome<()>, WebError>| match result {
                            Ok(MutationOutcome::Confirmed(())) => None,
                            Ok(MutationOutcome::CommitIndeterminate(())) => {
                                Some(
                                    view! {
                                        <p class="error">
                                            "Your sign-out may have succeeded, but its status could not be confirmed. Refresh to check."
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

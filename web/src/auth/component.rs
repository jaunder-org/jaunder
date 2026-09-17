//! The **auth** vertical's wasm-only UI (ADR-0070): `LoginPage` and `LogoutPage`.
//! Never host-compiled — free to call browser primitives (the advisory auth
//! [`marker_storage`](super::marker_storage) binding) directly, no `cfg` gates
//! inside this file.

use super::{Login, Logout, SessionUser};
use crate::app::PrivateDestination;
use crate::error::WebError;
use crate::forms::{self, Field, ValidatedInput};
use crate::passkeys;
use crate::topbar::Topbar;
use common::{MutationOutcome, password::PasswordShape, username::Username};
use leptos::prelude::*;
use leptos_router::{
    NavigateOptions,
    hooks::{use_navigate, use_query_map},
};

/// Login page.
#[component]
pub fn LoginPage() -> impl IntoView {
    let login_action = ServerAction::<Login>::new();
    let navigate = use_navigate();
    let query = use_query_map();

    // On a successful login, store the returned session directly: this updates the
    // reactive signal so the chrome flips without a document reload, and mirrors it
    // into the advisory marker (#181, ADR-0044) for the next pre-paint boot.
    Effect::new(move |_| {
        if let Some(Ok(outcome)) = login_action.value().get() {
            match outcome {
                MutationOutcome::Confirmed(session) => {
                    super::set_session(session);
                    super::use_session().reconcile.refetch();
                    let destination =
                        login_return_destination(query.get().get("return_to").as_deref());
                    navigate(&destination, NavigateOptions::default());
                }
                MutationOutcome::CommitIndeterminate(session) => {
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
                <PasskeyLogin />
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

/// Returns the Login flow's safe client-side destination.
fn login_return_destination(return_to: Option<&str>) -> String {
    PrivateDestination::parse(return_to.unwrap_or_default()).map_or_else(
        || "/app".to_owned(),
        |destination| destination.as_ref().to_owned(),
    )
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
            .zip(password.value().parse().ok())
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
/// Browser passkey sign-in. The cookie established by finish_authentication is
/// reconciled through the same shared session authority as password login.
#[component]
fn PasskeyLogin() -> impl IntoView {
    let navigate = use_navigate();
    let query = use_query_map();
    let availability = Resource::new(|| (), |()| passkeys::availability());
    let status = RwSignal::new(None::<String>);
    let working = RwSignal::new(false);
    let supported = client::webauthn::is_supported();
    let session_context = super::use_session();
    let sign_in = move |_| {
        if !supported || !matches!(availability.get_untracked(), Some(Ok(true))) {
            return;
        }
        let navigate = navigate.clone();
        working.set(true);
        status.set(None);
        leptos::task::spawn_local(async move {
            let message = match authenticate_passkey().await {
                PasskeyAuthentication::Confirmed(session) => {
                    session_context.set(session);
                    session_context.reconcile.refetch();
                    let destination =
                        login_return_destination(query.get().get("return_to").as_deref());
                    navigate(&destination, NavigateOptions::default());
                    "Signed in with your passkey.".to_owned()
                }
                PasskeyAuthentication::Indeterminate(session) => {
                    session_context.set(session);
                    session_context.reconcile.refetch();
                    "Your sign-in may have succeeded. Refresh to confirm your session.".to_owned()
                }
                PasskeyAuthentication::Message(message) => message,
            };
            working.set(false);
            status.set(Some(message));
        });
    };

    view! {
        <section class="j-card" data-test="passkey-login">
            <div class="j-card-head">
                <h2>"Sign in with a passkey"</h2>
            </div>
            <div class="j-form-body">
                <p>"Use a passkey from this device or a connected security key."</p>
                <PasskeyLoginAvailability availability=availability supported=supported />
            </div>
            <div class="j-form-actions">
                <button
                    type="button"
                    class="j-btn"
                    data-test="passkey-login-button"
                    prop:disabled=move || {
                        working.get() || !supported || !matches!(availability.get(), Some(Ok(true)))
                    }
                    on:click=sign_in
                >
                    {move || if working.get() { "Waiting for passkey…" } else { "Use a passkey" }}
                </button>
            </div>
            {move || {
                status
                    .get()
                    .map(|message| {
                        view! {
                            <p role="status" aria-live="polite" data-test="passkey-login-status">
                                {message}
                            </p>
                        }
                    })
            }}
        </section>
    }
}

#[component]
fn PasskeyLoginAvailability(
    availability: Resource<Result<bool, WebError>>,
    supported: bool,
) -> impl IntoView {
    view! {
        {move || match availability.get() {
            None => {
                view! {
                    <p role="status" aria-live="polite" data-test="passkey-login-loading">
                        "Checking passkey availability…"
                    </p>
                }
                    .into_any()
            }
            Some(Ok(false)) => {
                view! {
                    <p class="error" role="alert" data-test="passkey-login-site-unavailable">
                        "Passkeys are not available on this site."
                    </p>
                }
                    .into_any()
            }
            Some(Ok(true)) if !supported => {
                view! {
                    <p class="error" role="alert" data-test="passkey-login-unsupported">
                        "Passkeys are not supported by this browser."
                    </p>
                }
                    .into_any()
            }
            Some(Ok(true)) => ().into_any(),
            Some(Err(_)) => {
                view! {
                    <p class="error" role="alert" data-test="passkey-login-availability-failed">
                        "Passkey availability could not be checked. Try again."
                    </p>
                }
                    .into_any()
            }
        }}
    }
}

enum PasskeyAuthentication {
    Confirmed(SessionUser),
    Indeterminate(SessionUser),
    Message(String),
}

async fn authenticate_passkey() -> PasskeyAuthentication {
    // crap:allow: WebAuthn browser ceremony and generated client transport are wasm-only and covered by browser tests.
    let start = match passkeys::start_authentication().await {
        Ok(MutationOutcome::Confirmed(start)) => start,
        Ok(MutationOutcome::CommitIndeterminate(_)) => {
            return PasskeyAuthentication::Message(
                "Passkey sign-in could not be confirmed. Try again.".to_owned(),
            );
        }
        Err(error) => return PasskeyAuthentication::Message(error.to_string()),
    };
    let response = match client::webauthn::get(&start.request).await {
        client::webauthn::CeremonyOutcome::Success(response) => response,
        client::webauthn::CeremonyOutcome::Cancelled => {
            return PasskeyAuthentication::Message("Passkey sign-in cancelled.".to_owned());
        }
        client::webauthn::CeremonyOutcome::Unsupported => {
            return PasskeyAuthentication::Message(
                "Passkeys are not supported by this browser.".to_owned(),
            );
        }
        client::webauthn::CeremonyOutcome::Failed => {
            return PasskeyAuthentication::Message("Passkey sign-in failed. Try again.".to_owned());
        }
    };
    match passkeys::finish_authentication(start.handle, response).await {
        Ok(MutationOutcome::Confirmed(session)) => PasskeyAuthentication::Confirmed(session),
        Ok(MutationOutcome::CommitIndeterminate(session)) => {
            PasskeyAuthentication::Indeterminate(session)
        }
        Err(_) => {
            PasskeyAuthentication::Message("Passkey sign-in could not be completed.".to_owned())
        }
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

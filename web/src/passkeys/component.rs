//! Wasm-only passkey registration, credential management, and browser ceremony UI.

use crate::{
    forms::{Field, ValidatedInput},
    topbar::Topbar,
};
use common::{MutationOutcome, password::PasswordShape};
use leptos::prelude::*;
use leptos::task::spawn_local;

use super::page_state::{
    self, Availability, CredentialListState, DeletionState, RegistrationState,
};
use super::{BrowserPasskeyLabel, CredentialInfo};

/// Authenticated passkey registration and credential-management route.
#[component]
pub fn PasskeysPage() -> impl IntoView {
    let availability = Resource::new(|| (), |()| super::availability());
    let refresh = RwSignal::new(0_u32);
    let credentials = Resource::new(move || refresh.get(), |_| super::list());
    let browser_supported = client::webauthn::is_supported();
    let session = crate::auth::use_session().current;
    let registration_status = RwSignal::new(None::<String>);

    view! {
        <Topbar title="Passkeys".to_string() sub="Sign in securely with this device".to_string() />
        <div class="j-scroll">
            <div class="j-page" data-test="passkeys-page">
                {move || {
                    if session.get().is_none() {
                        view! {
                            <p class="error" role="alert" data-test="passkeys-auth-required">
                                "Sign in to manage passkeys."
                            </p>
                        }
                            .into_any()
                    } else {
                        view! {
                            {move || match page_state::availability(
                                availability.get().as_ref(),
                                browser_supported,
                            ) {
                                Availability::Loading => {
                                    view! {
                                        <p class="j-loading" data-test="passkeys-loading">
                                            "Checking passkey availability…"
                                        </p>
                                    }
                                        .into_any()
                                }
                                Availability::Unsupported => {
                                    view! {
                                        <p
                                            class="error"
                                            role="alert"
                                            data-test="passkeys-unsupported"
                                        >
                                            "Passkeys are not available in this browser or on this site."
                                        </p>
                                    }
                                        .into_any()
                                }
                                Availability::Failed => {
                                    view! {
                                        <p
                                            class="error"
                                            role="alert"
                                            data-test="passkeys-availability-failed"
                                        >
                                            "Passkey availability could not be checked. Try again."
                                        </p>
                                    }
                                        .into_any()
                                }
                                Availability::Available => {
                                    view! {
                                        <PasskeyRegistration
                                            refresh=refresh
                                            status=registration_status
                                        />
                                    }
                                        .into_any()
                                }
                            }}
                            {move || {
                                registration_status
                                    .get()
                                    .map(|message| {
                                        view! {
                                            <p
                                                role="status"
                                                aria-live="polite"
                                                data-test="passkey-registration-status"
                                            >
                                                {message}
                                            </p>
                                        }
                                    })
                            }}
                            <CredentialList credentials=credentials refresh=refresh />
                        }
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}

#[component]
fn PasskeyRegistration(refresh: RwSignal<u32>, status: RwSignal<Option<String>>) -> impl IntoView {
    let label = Field::<BrowserPasskeyLabel>::new();
    let password = Field::<PasswordShape>::new();
    let working = RwSignal::new(false);
    let submit = registration_submit(label, password, refresh, status, working);

    view! {
        <section class="j-card" data-test="passkey-registration">
            <div class="j-card-head">
                <h2>"Add a passkey"</h2>
            </div>
            <form class="j-form-body" on:submit=submit>
                <ValidatedInput<BrowserPasskeyLabel>
                    label="Passkey label"
                    name="passkey-label"
                    autocomplete="off"
                    field=label
                    placeholder="This device"
                />
                <ValidatedInput<PasswordShape>
                    label="Current password"
                    name="passkey-password"
                    input_type="password"
                    autocomplete="current-password"
                    field=password
                />
                <div class="j-form-actions">
                    <button
                        type="submit"
                        class="j-btn is-primary"
                        data-test="passkey-register"
                        prop:disabled=move || {
                            working.get() || !label.is_valid() || !password.is_valid()
                        }
                    >
                        {move || if working.get() { "Adding passkey…" } else { "Add passkey" }}
                    </button>
                </div>
            </form>
        </section>
    }
}

fn registration_submit(
    label: Field<BrowserPasskeyLabel>,
    password: Field<PasswordShape>,
    refresh: RwSignal<u32>,
    status: RwSignal<Option<String>>,
    working: RwSignal<bool>,
) -> impl Fn(leptos::ev::SubmitEvent) {
    move |event| {
        event.prevent_default();
        let Some((label, password)) = label.parsed().zip(password.value().parse().ok()) else {
            return;
        };
        working.set(true);
        status.set(None);
        spawn_local(async move {
            let feedback = match super::start_registration(label, password).await {
                Ok(MutationOutcome::Confirmed(start)) => {
                    match client::webauthn::create(&start.request).await {
                        client::webauthn::CeremonyOutcome::Success(response) => {
                            let handle: Result<super::CeremonyHandle, _> = start.handle.try_into();
                            match handle {
                                Ok(handle) => match super::finish_registration(handle, response)
                                    .await
                                {
                                    Ok(MutationOutcome::Confirmed(())) => RegistrationState::Added,
                                    Ok(MutationOutcome::CommitIndeterminate(())) => {
                                        RegistrationState::MayHaveAdded
                                    }
                                    Err(_) => RegistrationState::CouldNotSave,
                                },
                                Err(_) => RegistrationState::Invalid,
                            }
                        }
                        outcome => RegistrationState::Ceremony(page_state::ceremony_state(
                            ceremony_result(&outcome),
                        )),
                    }
                }
                Ok(MutationOutcome::CommitIndeterminate(_)) => RegistrationState::Indeterminate,
                Err(error) => {
                    working.set(false);
                    status.set(Some(error.to_string()));
                    return;
                }
            };
            let (message, should_refresh) = page_state::registration_feedback(feedback);
            if should_refresh {
                refresh.update(|version| *version += 1);
            }
            working.set(false);
            status.set(Some(message.to_owned()));
        });
    }
}

#[component]
fn CredentialList(
    credentials: Resource<Result<Vec<CredentialInfo>, crate::error::WebError>>,
    refresh: RwSignal<u32>,
) -> impl IntoView {
    view! {
        <section data-test="passkey-credentials">
            <h2>"Your passkeys"</h2>
            {move || match page_state::credential_list_state(credentials.get().as_ref()) {
                CredentialListState::Loading => {
                    view! { <p class="j-loading">"Loading passkeys…"</p> }.into_any()
                }
                CredentialListState::Empty => {
                    view! {
                        <p data-test="passkey-credentials-empty">"You have no passkeys yet."</p>
                    }
                        .into_any()
                }
                CredentialListState::Failed => {
                    view! {
                        <p class="error" role="alert">
                            "Passkeys could not be loaded."
                        </p>
                    }
                        .into_any()
                }
                CredentialListState::Ready => {
                    match credentials.get() {
                        Some(Ok(credentials)) => {
                            view! {
                                <ul data-test="passkey-credentials-list">
                                    {credentials
                                        .into_iter()
                                        .map(|credential| {
                                            view! {
                                                <CredentialRow credential=credential refresh=refresh />
                                            }
                                        })
                                        .collect::<Vec<_>>()}
                                </ul>
                            }
                                .into_any()
                        }
                        Some(Err(_)) | None => unreachable!("state fold checked readiness"),
                    }
                }
            }}
        </section>
    }
}

#[component]
fn CredentialRow(credential: CredentialInfo, refresh: RwSignal<u32>) -> impl IntoView {
    let password = Field::<PasswordShape>::new();
    let status = RwSignal::new(None::<String>);
    let working = RwSignal::new(false);
    let submit = deletion_submit(credential.id.clone(), password, refresh, status, working);
    let created_at = credential.created_at.to_string();
    let last_used = credential
        .last_used_at
        .map_or_else(|| "Never".to_owned(), |time| time.to_string());

    view! {
        <li class="j-card" data-test="passkey-credential">
            <div class="j-card-head">
                <h3>{credential.label}</h3>
            </div>
            <p>"Created: " {created_at}</p>
            <p>"Last used: " {last_used}</p>
            <form class="j-form-body" on:submit=submit>
                <ValidatedInput<PasswordShape>
                    label="Current password to remove this passkey"
                    name="passkey-delete-password"
                    input_type="password"
                    autocomplete="current-password"
                    field=password
                />
                <div class="j-form-actions">
                    <button
                        type="submit"
                        class="j-btn is-danger"
                        data-test="passkey-delete"
                        prop:disabled=move || working.get() || !password.is_valid()
                    >
                        {move || if working.get() { "Removing…" } else { "Remove passkey" }}
                    </button>
                </div>
            </form>
            {move || {
                status
                    .get()
                    .map(|message| {
                        view! {
                            <p role="status" aria-live="polite">
                                {message}
                            </p>
                        }
                    })
            }}
        </li>
    }
}

fn deletion_submit(
    credential_id: String,
    password: Field<PasswordShape>,
    refresh: RwSignal<u32>,
    status: RwSignal<Option<String>>,
    working: RwSignal<bool>,
) -> impl Fn(leptos::ev::SubmitEvent) {
    move |event| {
        event.prevent_default();
        let Some(password) = password.value().parse().ok() else {
            return;
        };
        let Ok(credential_id) = credential_id.clone().try_into() else {
            return;
        };
        working.set(true);
        spawn_local(async move {
            let feedback = match super::delete(credential_id, password).await {
                Ok(MutationOutcome::Confirmed(())) => DeletionState::Deleted,
                Ok(MutationOutcome::CommitIndeterminate(())) => DeletionState::MayHaveDeleted,
                Err(error) => {
                    working.set(false);
                    status.set(Some(error.to_string()));
                    return;
                }
            };
            let (message, should_refresh) = page_state::deletion_feedback(feedback);
            if should_refresh {
                refresh.update(|version| *version += 1);
            }
            working.set(false);
            status.set(Some(message.to_owned()));
        });
    }
}

fn ceremony_result<T>(
    outcome: &client::webauthn::CeremonyOutcome<T>,
) -> super::page_state::CeremonyResult {
    match outcome {
        client::webauthn::CeremonyOutcome::Success(_) => super::page_state::CeremonyResult::Success,
        client::webauthn::CeremonyOutcome::Cancelled => {
            super::page_state::CeremonyResult::Cancelled
        }
        client::webauthn::CeremonyOutcome::Unsupported => {
            super::page_state::CeremonyResult::Unsupported
        }
        client::webauthn::CeremonyOutcome::Failed => super::page_state::CeremonyResult::Failed,
    }
}

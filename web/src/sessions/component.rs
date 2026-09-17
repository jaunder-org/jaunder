use crate::forms::{Field, ValidatedInput};
use crate::topbar::Topbar;
use common::{MutationOutcome, session_label::SessionLabel};
use leptos::prelude::*;

use super::api::{self, CreateAppPassword, Revoke};

/// Sessions page — lists all sessions, mints app passwords, and revokes sessions.
#[component]
pub fn SessionsPage() -> impl IntoView {
    let revoke_action = ServerAction::<Revoke>::new();
    let create_action = ServerAction::<CreateAppPassword>::new();
    let sessions = Resource::new(
        move || (revoke_action.version().get(), create_action.version().get()),
        |_| api::list(),
    );

    view! {
        <Topbar title="Sessions" sub="Active sessions" />
        <div class="j-scroll">
            <div class="j-page">
                <AppPasswordCreator create_action=create_action />
                {move || {
                    revoke_action
                        .value()
                        .get()
                        .and_then(|result| {
                            match crate::mutation_feedback::classify(
                                result,
                                "The session may have been revoked, but its status could not be confirmed. Refresh to check.",
                            ) {
                                crate::mutation_feedback::MutationFeedback::Confirmed(()) => None,
                                crate::mutation_feedback::MutationFeedback::Error(message) => {
                                    Some(view! { <p class="error">{message}</p> }.into_any())
                                }
                            }
                        })
                }}
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match sessions.await {
                            Ok(list) => {
                                view! {
                                    <ul>
                                        {list
                                            .into_iter()
                                            .map(|s| {
                                                let token_hash = s.token_hash.clone();
                                                view! {
                                                    <li>
                                                        {s.label.to_string()} " — last used: "
                                                        {s.last_used_at.to_string()}
                                                        {s.is_current.then_some(view! { " (current)" })} " "
                                                        <button
                                                            type="button"
                                                            class="j-btn is-danger"
                                                            on:click=move |_| {
                                                                revoke_action
                                                                    .dispatch(Revoke {
                                                                        token_hash: token_hash.clone(),
                                                                    });
                                                            }
                                                        >
                                                            "Revoke"
                                                        </button>
                                                    </li>
                                                }
                                            })
                                            .collect::<Vec<_>>()}
                                    </ul>
                                }
                                    .into_any()
                            }
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
            </div>
        </div>
    }
}

/// The "App passwords" creation control: a client-validated (ADR-0065 direct-bind)
/// label field + a plain button that dispatches [`CreateAppPassword`], plus the
/// once-shown raw-token display. `create_action` is owned by the parent so its
/// version bump refreshes the session list.
#[component]
fn AppPasswordCreator(create_action: ServerAction<CreateAppPassword>) -> impl IntoView {
    // Required field: a pristine empty label is invalid, so "Create app password"
    // stays disabled until a valid label is typed.
    let label_field = Field::<SessionLabel>::new();

    view! {
        <section class="j-card" data-app-passwords>
            <div class="j-card-head">
                <div>
                    <h2>"App passwords"</h2>
                    <div class="j-sub">
                        "Create a password for an external AtomPub editor such as MarsEdit."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                <ValidatedInput<SessionLabel>
                    label="Label"
                    name="label"
                    field=label_field
                    placeholder="e.g. MarsEdit"
                />
            </div>
            <div class="j-form-actions">
                <button
                    type="button"
                    class="j-btn is-primary"
                    prop:disabled=move || !label_field.is_valid()
                    on:click=move |_| {
                        if let Some(label) = label_field.parsed() {
                            create_action.dispatch(CreateAppPassword { label });
                        }
                    }
                >
                    "Create app password"
                </button>
            </div>
            {move || {
                create_action
                    .value()
                    .get()
                    .map(|result| match result {
                        Ok(MutationOutcome::Confirmed(pw)) => {
                            view! {
                                <AppPasswordToken
                                    token=pw.token
                                    presentation=AppPasswordPresentation::Confirmed
                                />
                            }
                                .into_any()
                        }
                        Ok(MutationOutcome::CommitIndeterminate(pw)) => {
                            view! {
                                <AppPasswordToken
                                    token=pw.token
                                    presentation=AppPasswordPresentation::CommitIndeterminate
                                />
                            }
                                .into_any()
                        }
                        Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                    })
            }}
        </section>
    }
}

#[derive(Clone, Copy)]
enum AppPasswordPresentation {
    Confirmed,
    CommitIndeterminate,
}

impl AppPasswordPresentation {
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Self::Confirmed => (
                "success",
                "Copy this app password now \u{2014} it will not be shown again: ",
            ),
            Self::CommitIndeterminate => (
                "error",
                "The app password may have been created, but its status could not be confirmed. Copy it now and refresh to check: ",
            ),
        }
    }
}

/// One-time App Password presentation with an explicit clipboard action and manual fallback.
#[component]
fn AppPasswordToken(
    token: common::token::RawToken,
    presentation: AppPasswordPresentation,
) -> impl IntoView {
    let copied = RwSignal::new(false);
    let copy_error = RwSignal::new(None::<&'static str>);
    let copy_attempt = RwSignal::new(0_u64);
    let token_text = token.to_string();
    let copy_value = token_text.clone();
    let (status_class, prompt) = presentation.parts();

    view! {
        <div data-app-password-token>
            <p class=status_class>
                {prompt} <code>{token_text}</code> " "
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| {
                        copy_app_password(copy_value.clone(), copied, copy_error, copy_attempt);
                    }
                >
                    {move || if copied.get() { "Copied" } else { "Copy app password" }}
                </button>
            </p>
            {move || {
                copy_error
                    .get()
                    .map(|message| {
                        view! {
                            <p class="error" data-app-password-copy-error>
                                {message}
                            </p>
                        }
                    })
            }}
        </div>
    }
}

fn copy_app_password(
    token: String,
    copied: RwSignal<bool>,
    copy_error: RwSignal<Option<&'static str>>,
    copy_attempt: RwSignal<u64>,
) {
    use leptos::task::spawn_local;
    use leptos_dom::helpers::set_timeout;
    use std::time::Duration;

    let attempt = copy_attempt.get_untracked().wrapping_add(1);
    copy_attempt.set(attempt);
    spawn_local(async move {
        let succeeded = client::clipboard::write_text(&token).await.is_ok();
        if copy_attempt.get_untracked() != attempt {
            return;
        }
        if succeeded {
            copy_error.set(None);
            copied.set(true);
            set_timeout(
                move || {
                    if copy_attempt.get_untracked() == attempt {
                        copied.set(false);
                    }
                },
                Duration::from_secs(2),
            );
        } else {
            copied.set(false);
            copy_error.set(Some(
                "Could not copy the App Password. Select it manually instead.",
            ));
        }
    });
}

use crate::forms::{self, Field, ValidatedBareInput};
use crate::topbar::Topbar;
use common::session_label::SessionLabel;
use leptos::prelude::*;

use super::api::{CreateAppPassword, Revoke, list};

/// Sessions page — lists all sessions, mints app passwords, and revokes sessions.
#[component]
pub fn SessionsPage() -> impl IntoView {
    let revoke_action = ServerAction::<Revoke>::new();
    let create_action = ServerAction::<CreateAppPassword>::new();
    let sessions = Resource::new(
        move || (revoke_action.version().get(), create_action.version().get()),
        |_| list(),
    );

    view! {
        <Topbar title="Sessions" sub="Active sessions" />
        <div class="j-scroll">
            <div class="j-page">
                <AppPasswordCreator create_action=create_action />
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
        <section class="j-app-passwords">
            <h2>"App passwords"</h2>
            <p>
                "Create a password to publish from an external editor (such as MarsEdit) over AtomPub."
            </p>
            <label for="app-password-label">"Label"</label>
            <ValidatedBareInput<SessionLabel>
                name="label"
                field=label_field
                id=Some("app-password-label")
                placeholder=Some("Label (e.g. MarsEdit)")
            />
            {forms::validated_error(
                label_field.error,
                Signal::derive(move || label_field.is_touched()),
                |msg| view! { <p class="error">{msg}</p> }.into_any(),
            )}
            <button
                type="button"
                class="j-btn"
                prop:disabled=move || !label_field.is_valid()
                on:click=move |_| {
                    if let Some(label) = label_field.parsed() {
                        create_action.dispatch(CreateAppPassword { label });
                    }
                }
            >
                "Create app password"
            </button>
            {move || {
                create_action
                    .value()
                    .get()
                    .map(|result| match result {
                        Ok(pw) => {
                            view! {
                                <div class="j-app-password-token">
                                    <p>
                                        "Copy this app password now \u{2014} it will not be shown again:"
                                    </p>
                                    <code>{pw.token.to_string()}</code>
                                </div>
                            }
                                .into_any()
                        }
                        Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                    })
            }}
        </section>
    }
}

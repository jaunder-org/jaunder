use crate::pages::Topbar;
use crate::sessions::{list_sessions, CreateAppPassword, RevokeSession};
use leptos::prelude::*;

/// Sessions page — lists all sessions, mints app passwords, and revokes sessions.
#[allow(clippy::must_use_candidate)]
#[component]
pub fn SessionsPage() -> impl IntoView {
    let revoke_action = ServerAction::<RevokeSession>::new();
    let create_action = ServerAction::<CreateAppPassword>::new();
    let sessions = crate::server_resource(
        move || (revoke_action.version().get(), create_action.version().get()),
        |_| list_sessions(),
    );

    view! {
        <Topbar title="Sessions".to_string() sub="Active sessions".to_string() />
        <div class="j-scroll">
            <div class="j-page">
                <section class="j-app-passwords">
                    <h2>"App passwords"</h2>
                    <p>
                        "Create a password to publish from an external editor (such as MarsEdit) over AtomPub."
                    </p>
                    <ActionForm action=create_action>
                        <input
                            type="text"
                            name="label"
                            placeholder="Label (e.g. MarsEdit)"
                            required
                        />
                        <button type="submit" class="j-btn">
                            "Create app password"
                        </button>
                    </ActionForm>
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
                                            <code>{pw.token}</code>
                                        </div>
                                    }
                                        .into_any()
                                }
                                Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                            })
                    }}
                </section>
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
                                                let hash = s.token_hash.clone();
                                                view! {
                                                    <li>
                                                        {s.label.clone()} " — last used: "
                                                        {s.last_used_at.clone()}
                                                        {s.is_current.then_some(view! { " (current)" })} " "
                                                        <ActionForm action=revoke_action>
                                                            <input type="hidden" name="token_hash" value=hash />
                                                            <button type="submit" class="j-btn is-danger">
                                                                "Revoke"
                                                            </button>
                                                        </ActionForm>
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

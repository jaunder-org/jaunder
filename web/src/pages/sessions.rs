use crate::sessions::{list_sessions, RevokeSession};
use leptos::prelude::*;

/// Sessions page — lists all sessions; allows revoking individual sessions.
#[component]
pub fn SessionsPage() -> impl IntoView {
    let revoke_action = ServerAction::<RevokeSession>::new();
    let sessions = Resource::new(move || revoke_action.version().get(), |_| list_sessions());

    view! {
        <h1>"Sessions"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
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
                                                {s
                                                    .label
                                                    .clone()
                                                    .unwrap_or_else(|| "(no label)".to_string())}
                                                " — last used: " {s.last_used_at.clone()}
                                                {s.is_current.then_some(view! { " (current)" })} " "
                                                <ActionForm action=revoke_action>
                                                    <input type="hidden" name="token_hash" value=hash />
                                                    <button type="submit">"Revoke"</button>
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
    }
}

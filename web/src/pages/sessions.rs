use crate::pages::Topbar;
use crate::sessions::{list_sessions, RevokeSession};
use leptos::prelude::*;

/// Sessions page — lists all sessions; allows revoking individual sessions.
#[allow(clippy::must_use_candidate)]
#[component]
pub fn SessionsPage() -> impl IntoView {
    let revoke_action = ServerAction::<RevokeSession>::new();
    let sessions = Resource::new(move || revoke_action.version().get(), |_| list_sessions());

    view! {
        <Topbar title="Sessions".to_string() sub="Active sessions".to_string() />
        <div class="j-scroll">
            <div class="j-page">
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

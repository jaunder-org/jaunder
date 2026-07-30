use super::{is_subscribed, Subscribe, Unsubscribe};
use common::username::Username;
use leptos::prelude::*;

/// Subscribe / Unsubscribe control shown on a user's profile (timeline) page.
///
/// Hidden when the viewer is logged out or is viewing their own profile.
/// Otherwise renders Subscribe when not subscribed and Unsubscribe when
/// subscribed, querying state via `is_subscribed`.
#[component]
pub fn SubscribeButton(username: Username) -> impl IntoView {
    let subscribe = ServerAction::<Subscribe>::new();
    let unsubscribe = ServerAction::<Unsubscribe>::new();

    // Re-query the subscription after either action mutates it; the viewer identity
    // comes from the shared session (stable, so it needs no per-action re-query) (#591).
    let session = crate::auth::use_session();
    let username_for_state = username.clone();
    let state = Resource::new(
        move || (subscribe.version().get(), unsubscribe.version().get()),
        move |_| {
            let username = username_for_state.clone();
            async move {
                let subscribed = is_subscribed(username.clone()).await.unwrap_or(false);
                (
                    session.current.get_untracked().map(|u| u.username),
                    subscribed,
                )
            }
        },
    );

    let profile_username = username;

    view! {
        <Suspense fallback=|| ()>
            {move || {
                let username = profile_username.clone();
                Suspend::new(async move {
                    let (viewer, subscribed) = state.await;
                    let show = match &viewer {
                        Some(name) => *name != username,
                        None => false,
                    };
                    if !show {
                        return ().into_any();
                    }
                    if subscribed {
                        view! {
                            <ActionForm action=unsubscribe>
                                <input
                                    type="hidden"
                                    name="author_username"
                                    value=username.to_string()
                                />
                                <button type="submit" class="j-btn">
                                    "Unsubscribe"
                                </button>
                            </ActionForm>
                        }
                            .into_any()
                    } else {
                        view! {
                            <ActionForm action=subscribe>
                                <input
                                    type="hidden"
                                    name="author_username"
                                    value=username.to_string()
                                />
                                <button type="submit" class="j-btn is-primary">
                                    "Subscribe"
                                </button>
                            </ActionForm>
                        }
                            .into_any()
                    }
                })
            }}
        </Suspense>
    }
}

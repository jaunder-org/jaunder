use super::state::{self, SubscribePaint};
use super::{Subscribe, Unsubscribe};
use crate::auth;
use common::{MutationOutcome, username::Username};
use leptos::prelude::*;

/// Subscribe / Unsubscribe control shown on a user's profile (timeline) page.
///
/// Hidden when the viewer is logged out or is viewing their own profile. Otherwise
/// renders Subscribe when not subscribed, Unsubscribe when subscribed, and an error
/// when the subscription state could not be determined.
///
/// The three-way choice is [`paint`], host-tested in `state.rs`: this file is
/// wasm-only, so a decision made here could not be asserted anywhere (#306).
#[component]
pub fn SubscribeButton(username: Username) -> impl IntoView {
    let subscribe = ServerAction::<Subscribe>::new();
    let unsubscribe = ServerAction::<Unsubscribe>::new();

    // Re-query the subscription after either action mutates it; the viewer identity
    // comes from the shared session (stable, so it needs no per-action re-query) (#591).
    let session = auth::use_session();
    let username_for_state = username.clone();
    let state = Resource::new(
        move || (subscribe.version().get(), unsubscribe.version().get()),
        move |_| {
            let username = username_for_state.clone();
            async move {
                // The error is carried, not collapsed — see `state::paint` (#861).
                let subscribed = super::is_subscribed(username.clone()).await;
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
                    match state::paint(viewer.as_ref(), &username, subscribed) {
                        SubscribePaint::Hidden => ().into_any(),
                        SubscribePaint::Toggle(subscribed) => {
                            view! {
                                <SubscriptionToggle
                                    username=username
                                    subscribed=subscribed
                                    subscribe=subscribe
                                    unsubscribe=unsubscribe
                                />
                            }
                                .into_any()
                        }
                        SubscribePaint::Failed(err) => {
                            view! { <p class="error">{err.to_string()}</p> }.into_any()
                        }
                    }
                })
            }}
        </Suspense>
    }
}

/// The one form the resolved state paints: `Unsubscribe` when the viewer already
/// subscribes to `username`, `Subscribe` otherwise.
///
/// Split out of [`SubscribeButton`] (#306) so the parent's `view!` carries only the
/// "is there anything to show at all" decision; this component owns the
/// subscribed/not-subscribed branch.
#[component]
fn SubscriptionToggle(
    /// The profile author the form targets.
    username: Username,
    /// Whether the viewer is already subscribed to `username`.
    subscribed: bool,
    subscribe: ServerAction<Subscribe>,
    unsubscribe: ServerAction<Unsubscribe>,
) -> impl IntoView {
    view! {
        {if subscribed {
            view! {
                <ActionForm action=unsubscribe>
                    <input type="hidden" name="author_username" value=username.to_string() />
                    <button type="submit" class="j-btn">
                        "Unsubscribe"
                    </button>
                </ActionForm>
            }
                .into_any()
        } else {
            view! {
                <ActionForm action=subscribe>
                    <input type="hidden" name="author_username" value=username.to_string() />
                    <button type="submit" class="j-btn is-primary">
                        "Subscribe"
                    </button>
                </ActionForm>
            }
                .into_any()
        }}
        <SubscriptionActionError subscribe=subscribe unsubscribe=unsubscribe />
    }
}

/// A failed subscribe/unsubscribe, rendered where the button is.
///
/// Without this the mutation is silent: only its `version()` bump re-runs the query,
/// so a failed unsubscribe repaints a Subscribe button and reads as success. That is
/// the other half of #861 — the toggle flipping is never proof the write landed.
#[component]
fn SubscriptionActionError(
    subscribe: ServerAction<Subscribe>,
    unsubscribe: ServerAction<Unsubscribe>,
) -> impl IntoView {
    view! {
        {move || {
            let outcome = subscribe.value().get().or_else(|| unsubscribe.value().get());
            match outcome {
                Some(Err(error)) => {
                    Some(view! { <p class="error">{error.to_string()}</p> }.into_any())
                }
                Some(Ok(MutationOutcome::CommitIndeterminate(()))) => {
                    Some(
                        view! {
                            <p class="error">
                                "Your subscription may have changed, but its status could not be confirmed. Refresh to check."
                            </p>
                        }
                            .into_any(),
                    )
                }
                Some(Ok(MutationOutcome::Confirmed(()))) | None => None,
            }
        }}
    }
}

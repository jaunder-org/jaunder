use leptos::prelude::*;

use crate::feed_discovery::FeedDiscovery;
use crate::pages::signal_read::read_signal;
use crate::pages::ui::{PostCard, Topbar};
use crate::posts::{list_local_timeline, TimelinePostSummary};
use common::feed::FeedSurface;

#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;

#[allow(clippy::too_many_lines)]
#[allow(clippy::must_use_candidate)]
#[component]
pub fn HomePage() -> impl IntoView {
    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    let next_cursor_created_at = RwSignal::new(None::<String>);
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    let next_cursor_post_id = RwSignal::new(None::<i64>);
    let has_more = RwSignal::new(false);
    let loading_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    // Public projector seed (#178/#179): `/` is the anonymous site (Local)
    // timeline for EVERYONE, including the authenticated owner — the owner stays on
    // this enhanced public front page (#181, ADR-0043 D10) rather than swapping to a
    // personalized feed (a content swap can't be flash-free; the projector paints
    // anonymous-only bytes). The personalized Feed lives at the `/app` cockpit.
    // Adopt the seed as the initial state so first paint shows content, no swap.
    if let Some(crate::render::PageSeed::SiteTimeline(page)) =
        leptos::prelude::use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        next_cursor_created_at.set(page.next_cursor_created_at);
        next_cursor_post_id.set(page.next_cursor_post_id);
        has_more.set(page.has_more);
        timeline.set(page.posts);
    }

    let refresh_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| refresh_version.update(|v| *v += 1));

    // The Local timeline is identical for every viewer, so the fetch is
    // viewer-independent — no `current_user()` gate and no mode swap (#181, D10).
    // Re-fetch on mutation (`refresh_version`) so the owner's own edits/deletes,
    // performed via the client-side action column, reflect immediately.
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    let initial_page = crate::server_resource(
        move || refresh_version.get(),
        |_| list_local_timeline(None, None, Some(50)),
    );

    // Client-only effect copies the resolved Resource into the mutation signals.
    // Plain `Effect::new` (not isomorphic): on the server the Resource future can
    // resolve after the per-request reactive owner is disposed, and an isomorphic
    // effect firing then would touch disposed signals and panic.
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(page) => {
                    timeline.set(page.posts);
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    loading_more.set(false);
                    if error.get_untracked().is_some() {
                        error.set(None);
                    }
                }
                Err(err) => {
                    error.set(Some(err.to_string()));
                    timeline.set(Vec::new());
                    has_more.set(false);
                }
            }
        }
    });

    let on_load_more = move |_| {
        #[cfg(not(target_arch = "wasm32"))]
        {}

        #[cfg(target_arch = "wasm32")]
        {
            if loading_more.get_untracked() || !has_more.get_untracked() {
                return;
            }

            loading_more.set(true);
            let cursor_created_at = next_cursor_created_at.get_untracked();
            let cursor_post_id = next_cursor_post_id.get_untracked();

            let timeline = timeline;
            let next_cursor_created_at_signal = next_cursor_created_at;
            let next_cursor_post_id_signal = next_cursor_post_id;
            let has_more_signal = has_more;
            let loading_more_signal = loading_more;
            let error_signal = error;

            spawn_local(async move {
                let result = list_local_timeline(cursor_created_at, cursor_post_id, Some(50)).await;
                match result {
                    Ok(page) => {
                        timeline.update(|rows| rows.extend(page.posts));
                        next_cursor_created_at_signal.set(page.next_cursor_created_at);
                        next_cursor_post_id_signal.set(page.next_cursor_post_id);
                        has_more_signal.set(page.has_more);
                        error_signal.set(None);
                    }
                    Err(err) => error_signal.set(Some(err.to_string())),
                }
                loading_more_signal.set(false);
            });
        }
    };

    let read_error = move || read_signal!(error);
    let read_timeline_rows = move || read_signal!(timeline);
    let read_has_more = move || read_signal!(has_more);
    let read_loading_more = move || read_signal!(loading_more);

    let load_more_btn = move || {
        read_has_more().then(|| {
            view! {
                <button on:click=on_load_more disabled=read_loading_more>
                    {move || if read_loading_more() { "Loading\u{2026}" } else { "Load more" }}
                </button>
            }
        })
    };

    view! {
        <FeedDiscovery surface=FeedSurface::Site />
        {move || {
            if let Some(err) = read_error() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            // Single-mode Local (#181, D10): `/` is always the enhanced public
            // timeline. The owner's own posts gain the client-side action column
            // (see `PostCard`'s marker match); the anon-only Sign in / Register CTA
            // is hidden for the owner via `html.authed` CSS (pre-paint, flash-free).
            view! {
                <Topbar
                    title="jaunder.local".to_string()
                    sub="Read-only \u{00b7} posts originating on this instance".to_string()
                >
                    <a href="/login" class="j-btn j-anon-only">
                        "Sign in"
                    </a>
                    <a href="/register" class="j-btn is-primary j-anon-only">
                        "Register"
                    </a>
                </Topbar>
                <div class="j-hero">
                    <h1>"One timeline. Every protocol."</h1>
                    <p>
                        "Jaunder is a self-hosted social client that reads from "
                        "ActivityPub, AT Protocol, RSS, Atom, and JSON Feed \u{2014} and "
                        "publishes back out to the ones you choose. "
                        "Below: what\u{2019}s been posted from this instance."
                    </p>
                </div>
                <div class="j-scroll">
                    {move || {
                        let rows = read_timeline_rows();
                        if rows.is_empty() {
                            view! { <p>"No posts yet."</p> }.into_any()
                        } else {
                            rows.into_iter()
                                .map(|p| {
                                    view! { <PostCard post=p banner=None on_mutate=on_mutate /> }
                                })
                                .collect::<Vec<_>>()
                                .into_any()
                        }
                    }} {load_more_btn}
                </div>
            }
                .into_any()
        }}
    }
}

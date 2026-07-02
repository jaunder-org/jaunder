//! The `/app` cockpit (#181, ADR-0043 D6): the authenticated owner's personalized
//! home Feed, relocated off `/` (which stays the enhanced public timeline, D10). A
//! first-class, directly-bookmarkable authed-only route — served from the SPA
//! shell (`no-store`), pre-painted `html.authed`, so a direct hit boots straight
//! into the feed with zero clicks. An anonymous / expired visitor bounces to
//! `/login`. This is the former `home.rs` Feed branch moved to its proper home.

use leptos::prelude::*;
use leptos_router::components::Redirect;

use crate::auth::current_user;
use crate::pages::signal_read::read_signal;
use crate::pages::ui::{InlineComposer, PostCard, Topbar};
use crate::posts::{list_home_feed, TimelinePostSummary};

#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;

#[allow(clippy::too_many_lines)]
#[allow(clippy::must_use_candidate)]
#[component]
pub fn CockpitPage() -> impl IntoView {
    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    let next_cursor_created_at = RwSignal::new(None::<String>);
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    let next_cursor_post_id = RwSignal::new(None::<i64>);
    let has_more = RwSignal::new(false);
    let loading_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let username = RwSignal::new(None::<String>);
    let bounce = RwSignal::new(false);

    let refresh_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| refresh_version.update(|v| *v += 1));

    // Gate on `current_user`, then fetch the personalized feed. Unlike `/`, `/app`
    // is authed-only and served from the SPA shell (no-store), so an async gate is
    // correct here — there is no cacheable-page flash constraint. `Ok(None)` means
    // anonymous / expired → bounce to `/login` (D6).
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    let initial_page = crate::server_resource(
        move || refresh_version.get(),
        |_| async move {
            match current_user().await {
                Ok(Some(user)) => list_home_feed(None, None, Some(50))
                    .await
                    .map(|page| Some((user, page))),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        },
    );

    // Client-only effect copies the resolved Resource into the mutation signals
    // (plain `Effect::new`, not isomorphic — the server future can resolve after
    // the per-request owner is disposed).
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(Some((user, page))) => {
                    username.set(Some(user));
                    timeline.set(page.posts);
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    loading_more.set(false);
                    if error.get_untracked().is_some() {
                        error.set(None);
                    }
                }
                Ok(None) => bounce.set(true),
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
                let result = list_home_feed(cursor_created_at, cursor_post_id, Some(50)).await;
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
    let read_bounce = move || read_signal!(bounce);
    let read_username = move || read_signal!(username);
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
        {move || {
            if read_bounce() {
                return view! { <Redirect path="/login" /> }.into_any();
            }
            if let Some(err) = read_error() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            match read_username() {
                None => {
                    view! {
                        <Topbar title="Home".to_string() />
                        <p class="j-loading">"Loading\u{2026}"</p>
                    }
                        .into_any()
                }
                Some(user) => {
                    view! {
                        <Topbar title="Home".to_string() sub="Your home feed".to_string() />
                        <InlineComposer username=user on_publish=refresh_version.write_only() />
                        <div class="j-scroll">
                            {move || {
                                let rows = read_timeline_rows();
                                if rows.is_empty() {
                                    view! { <p>"No posts yet."</p> }.into_any()
                                } else {
                                    rows.into_iter()
                                        .map(|p| {
                                            view! {
                                                <PostCard post=p banner=None on_mutate=on_mutate />
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .into_any()
                                }
                            }} {load_more_btn}
                        </div>
                    }
                        .into_any()
                }
            }
        }}
    }
}

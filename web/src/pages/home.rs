use leptos::prelude::*;

use crate::pages::signal_read::read_signal;
use crate::pages::ui::{InlineComposer, PostCard, Topbar};
use crate::posts::TimelinePostSummary;

#[cfg(target_arch = "wasm32")]
use crate::{
    auth::current_user,
    posts::{list_home_feed, list_local_timeline},
};
#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone)]
enum TimelineMode {
    Local,
    Feed(String),
}

#[component]
pub fn HomePage() -> impl IntoView {
    let timeline_mode = RwSignal::new(None::<TimelineMode>);
    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let _next_cursor_created_at = RwSignal::new(None::<String>);
    let _next_cursor_post_id = RwSignal::new(None::<i64>);
    let has_more = RwSignal::new(false);
    let loading_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    #[cfg(target_arch = "wasm32")]
    {
        let timeline_mode = timeline_mode;
        let timeline = timeline;
        let next_cursor_created_at = _next_cursor_created_at;
        let next_cursor_post_id = _next_cursor_post_id;
        let has_more = has_more;
        let error = error;
        spawn_local(async move {
            match current_user().await {
                Ok(Some(username)) => {
                    timeline_mode.set(Some(TimelineMode::Feed(username)));
                    match list_home_feed(None, None, Some(50)).await {
                        Ok(page) => {
                            timeline.set(page.posts);
                            next_cursor_created_at.set(page.next_cursor_created_at);
                            next_cursor_post_id.set(page.next_cursor_post_id);
                            has_more.set(page.has_more);
                            error.set(None);
                        }
                        Err(err) => error.set(Some(err.to_string())),
                    }
                }
                Ok(None) => {
                    timeline_mode.set(Some(TimelineMode::Local));
                    match list_local_timeline(None, None, Some(50)).await {
                        Ok(page) => {
                            timeline.set(page.posts);
                            next_cursor_created_at.set(page.next_cursor_created_at);
                            next_cursor_post_id.set(page.next_cursor_post_id);
                            has_more.set(page.has_more);
                            error.set(None);
                        }
                        Err(err) => error.set(Some(err.to_string())),
                    }
                }
                Err(err) => error.set(Some(err.to_string())),
            }
        });
    }

    let on_load_more = move |_| {
        #[cfg(not(target_arch = "wasm32"))]
        {}

        #[cfg(target_arch = "wasm32")]
        {
            if loading_more.get_untracked() || !has_more.get_untracked() {
                return;
            }
            let Some(mode) = timeline_mode.get_untracked() else {
                return;
            };

            loading_more.set(true);
            let cursor_created_at = _next_cursor_created_at.get_untracked();
            let cursor_post_id = _next_cursor_post_id.get_untracked();

            let timeline = timeline;
            let next_cursor_created_at_signal = _next_cursor_created_at;
            let next_cursor_post_id_signal = _next_cursor_post_id;
            let has_more_signal = has_more;
            let loading_more_signal = loading_more;
            let error_signal = error;

            spawn_local(async move {
                let result = match mode {
                    TimelineMode::Local => {
                        list_local_timeline(cursor_created_at, cursor_post_id, Some(50)).await
                    }
                    TimelineMode::Feed(_) => {
                        list_home_feed(cursor_created_at, cursor_post_id, Some(50)).await
                    }
                };

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
    let read_timeline_mode = move || read_signal!(timeline_mode);
    let read_timeline_rows = move || read_signal!(timeline);
    let read_has_more = move || read_signal!(has_more);
    let read_loading_more = move || read_signal!(loading_more);

    view! {
        {move || {
            if let Some(err) = read_error() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            match read_timeline_mode() {
                None => {
                    view! {
                        <Topbar title="Home".to_string() />
                        <p class="j-loading">"Loading timeline\u{2026}"</p>
                    }
                        .into_any()
                }
                Some(TimelineMode::Feed(username)) => {
                    let rows = read_timeline_rows();
                    let is_empty = rows.is_empty();
                    view! {
                        <Topbar title="Home".to_string() sub="Your home feed".to_string() />
                        <InlineComposer username=username />
                        <div class="j-scroll">
                            {if is_empty {
                                view! { <p>"No posts yet."</p> }.into_any()
                            } else {
                                rows.into_iter()
                                    .map(|p| view! { <PostCard post=p /> })
                                    .collect::<Vec<_>>()
                                    .into_any()
                            }}
                            {move || {
                                read_has_more()
                                    .then(|| {
                                        view! {
                                            <button on:click=on_load_more disabled=read_loading_more>
                                                {move || {
                                                    if read_loading_more() {
                                                        "Loading\u{2026}"
                                                    } else {
                                                        "Load more"
                                                    }
                                                }}
                                            </button>
                                        }
                                    })
                            }}
                        </div>
                    }
                        .into_any()
                }
                Some(TimelineMode::Local) => {
                    let rows = read_timeline_rows();
                    let is_empty = rows.is_empty();
                    view! {
                        <Topbar
                            title="jaunder.local".to_string()
                            sub="Read-only \u{00b7} posts originating on this instance".to_string()
                        >
                            <a href="/login" class="j-btn">
                                "Sign in"
                            </a>
                            <a href="/register" class="j-btn is-primary">
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
                            {if is_empty {
                                view! { <p>"No posts yet."</p> }.into_any()
                            } else {
                                rows.into_iter()
                                    .map(|p| view! { <PostCard post=p /> })
                                    .collect::<Vec<_>>()
                                    .into_any()
                            }}
                            {move || {
                                read_has_more()
                                    .then(|| {
                                        view! {
                                            <button on:click=on_load_more disabled=read_loading_more>
                                                {move || {
                                                    if read_loading_more() {
                                                        "Loading\u{2026}"
                                                    } else {
                                                        "Load more"
                                                    }
                                                }}
                                            </button>
                                        }
                                    })
                            }}
                        </div>
                    }
                        .into_any()
                }
            }
        }}
    }
}

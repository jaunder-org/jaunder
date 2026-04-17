use leptos::prelude::*;

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

    view! {
        <section>
            <h1>"Jaunder"</h1>
            <p>"A self-hosted social reader."</p>
            <nav>
                <a href="/login">"Login"</a>
                " "
                <a href="/register">"Register"</a>
            </nav>
            {move || {
                if let Some(err) = error.get() {
                    return view! { <p class="error">{err}</p> }.into_any();
                }
                let Some(mode) = timeline_mode.get() else {
                    return view! { <p>"Loading timeline..."</p> }.into_any();
                };
                let heading = match mode {
                    TimelineMode::Local => "Local Timeline".to_string(),
                    TimelineMode::Feed(ref username) => format!("Your Home Feed ({username})"),
                };
                let rows = timeline.get();
                let empty_message = match mode {
                    TimelineMode::Local => "No posts yet.",
                    TimelineMode::Feed(_) => "You have no published posts yet.",
                };
                if rows.is_empty() {
                    return view! {
                        <h2>{heading.clone()}</h2>
                        <p>{empty_message}</p>
                    }
                        .into_any();
                }

                view! {
                    <h2>{heading}</h2>
                    <ul>{rows.into_iter().map(render_timeline_post_row).collect::<Vec<_>>()}</ul>
                    {move || {
                        has_more
                            .get()
                            .then(|| {
                                view! {
                                    <button
                                        on:click=on_load_more
                                        disabled=move || loading_more.get()
                                    >
                                        {move || {
                                            if loading_more.get() { "Loading..." } else { "Load more" }
                                        }}
                                    </button>
                                }
                            })
                    }}
                }
                    .into_any()
            }}
        </section>
    }
}

fn render_timeline_post_row(post: TimelinePostSummary) -> impl IntoView {
    let author_href = format!("/~{}/", post.username);
    view! {
        <li data-test="timeline-item">
            <h3>
                <a href=post.permalink.clone()>{post.title}</a>
            </h3>
            <p class="metadata">
                "By " <a href=author_href>{post.username.clone()}</a> " on " {post.published_at}
            </p>
            <div class="content" inner_html=post.rendered_html></div>
        </li>
    }
}

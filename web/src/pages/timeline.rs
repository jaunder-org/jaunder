//! Shared paginated-timeline machinery (#181 review): the mutation signals, their
//! seed/resolve/fail transitions, wasm load-more, and the rows+load-more view —
//! used by the public Local timeline (`home.rs`) and the authed `/app` cockpit
//! (`cockpit.rs`) so the two pages don't duplicate it.

use leptos::prelude::*;

use crate::pages::signal_read::read_signal;
use crate::pages::ui::PostCard;
use crate::posts::{TimelinePage, TimelinePostSummary};

#[cfg(target_arch = "wasm32")]
use {crate::error::WebResult, leptos::task::spawn_local, std::future::Future};

/// The fetch page size shared by both timelines' initial + load-more requests.
pub(crate) const PAGE_SIZE: u32 = 50;

/// The reactive state of a cursor-paginated timeline — the six signals both pages
/// previously declared verbatim, plus the transitions over them.
#[derive(Clone, Copy)]
pub(crate) struct TimelineState {
    pub rows: RwSignal<Vec<TimelinePostSummary>>,
    pub next_cursor_created_at: RwSignal<Option<String>>,
    pub next_cursor_post_id: RwSignal<Option<i64>>,
    pub has_more: RwSignal<bool>,
    pub loading_more: RwSignal<bool>,
    pub error: RwSignal<Option<String>>,
}

impl Default for TimelineState {
    // cov:ignore-start
    fn default() -> Self {
        Self {
            rows: RwSignal::new(Vec::new()),
            next_cursor_created_at: RwSignal::new(None),
            next_cursor_post_id: RwSignal::new(None),
            has_more: RwSignal::new(false),
            loading_more: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }
    // cov:ignore-stop
}

impl TimelineState {
    /// Adopt the cursors + rows of `page` (a projector seed or a fresh fetch).
    // cov:ignore-start
    pub fn adopt(&self, page: TimelinePage) {
        self.next_cursor_created_at.set(page.next_cursor_created_at);
        self.next_cursor_post_id.set(page.next_cursor_post_id);
        self.has_more.set(page.has_more);
        self.rows.set(page.posts);
    }
    // cov:ignore-stop

    /// Resolve a re-fetch into the signals, clearing loading + any prior error.
    /// wasm-only: re-fetches resolve on the client, in the page's post-hydration
    /// `Effect` (the host build only ever runs the seed `adopt`).
    #[cfg(target_arch = "wasm32")]
    pub fn resolve(&self, page: TimelinePage) {
        self.adopt(page);
        self.loading_more.set(false);
        if self.error.get_untracked().is_some() {
            self.error.set(None);
        }
    }

    /// Record a fetch failure (empty the rows so a stale page isn't shown). wasm-only
    /// for the same reason as [`resolve`](Self::resolve).
    #[cfg(target_arch = "wasm32")]
    pub fn fail(&self, message: String) {
        self.error.set(Some(message));
        self.rows.set(Vec::new());
        self.has_more.set(false);
    }
}

/// wasm-only load-more: fetch the next page with the current cursors and append
/// it. `fetch` is the page's list fn (`list_local_timeline` / `list_home_feed`).
#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_load_more<F, Fut>(state: TimelineState, fetch: F)
where
    F: FnOnce(Option<String>, Option<i64>, Option<u32>) -> Fut + 'static,
    Fut: Future<Output = WebResult<TimelinePage>> + 'static,
{
    if state.loading_more.get_untracked() || !state.has_more.get_untracked() {
        return;
    }
    state.loading_more.set(true);
    let created_at = state.next_cursor_created_at.get_untracked();
    let post_id = state.next_cursor_post_id.get_untracked();
    spawn_local(async move {
        match fetch(created_at, post_id, Some(PAGE_SIZE)).await {
            Ok(page) => {
                state.rows.update(|rows| rows.extend(page.posts));
                state
                    .next_cursor_created_at
                    .set(page.next_cursor_created_at);
                state.next_cursor_post_id.set(page.next_cursor_post_id);
                state.has_more.set(page.has_more);
                state.error.set(None);
            }
            Err(err) => state.error.set(Some(err.to_string())),
        }
        state.loading_more.set(false);
    });
}

/// The scroll region shared by both timelines: the post list (or an empty
/// placeholder) followed by the load-more button.
#[component]
pub(crate) fn TimelineRows(
    state: TimelineState,
    on_mutate: Callback<()>,
    on_load_more: Callback<()>,
) -> impl IntoView {
    let read_rows = move || read_signal!(state.rows);
    let read_has_more = move || read_signal!(state.has_more);
    let read_loading_more = move || read_signal!(state.loading_more);
    view! {
        <div class="j-scroll">
            {move || {
                let rows = read_rows();
                if rows.is_empty() {
                    view! { <p>"No posts yet."</p> }.into_any()
                } else {
                    rows.into_iter()
                        .map(|p| view! { <PostCard post=p banner=None on_mutate=on_mutate /> })
                        .collect::<Vec<_>>()
                        .into_any()
                }
            }}
            {move || {
                read_has_more()
                    .then(|| {
                        view! {
                            <button
                                on:click=move |_| on_load_more.run(())
                                disabled=read_loading_more
                            >
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
}

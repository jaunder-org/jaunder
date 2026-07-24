//! Timeline pagination — the wasm-only reactive layer (ADR-0070): the
//! `TimelineState` signal bundle, the load-more task, and the shared
//! `TimelineRows` view. Its pure types + fold logic live in the ungated,
//! host-tested `state.rs`; this file carries no cfg gates of its own (its `mod`
//! declaration is `#[cfg(target_arch = "wasm32")]`).

use std::future::Future;

use leptos::prelude::*;
use leptos::task::spawn_local;

use common::ids::PostId;
use common::pagination::PageSize;
use common::seed::{TimelinePage, TimelinePostSummary};
use common::time::UtcInstant;

use super::state::{LoadStatus, TimelineCursor};
use crate::error::WebResult;
use crate::pages::signal_read::read_signal;
use crate::posts::PostCard;

/// The reactive state of a cursor-paginated timeline, shared by the public Local
/// timeline (`home.rs`) and the authed `/app` cockpit (`cockpit.rs`).
#[derive(Clone, Copy)]
pub struct TimelineState {
    pub rows: RwSignal<Vec<TimelinePostSummary>>,
    pub cursor: RwSignal<Option<TimelineCursor>>,
    pub has_more: RwSignal<bool>,
    pub status: RwSignal<LoadStatus>,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            rows: RwSignal::new(Vec::new()),
            cursor: RwSignal::new(None),
            has_more: RwSignal::new(false),
            status: RwSignal::new(LoadStatus::Idle),
        }
    }
}

impl TimelineState {
    /// Adopt a page's rows + cursor (a projector seed or a fresh fetch),
    /// replacing what's shown.
    pub fn adopt(&self, page: TimelinePage) {
        self.cursor.set(TimelineCursor::from_page(&page));
        self.has_more.set(page.has_more);
        self.rows.set(page.posts);
    }

    /// Resolve a re-fetch into the signals and settle to idle (clearing any prior
    /// failure). wasm-only: re-fetches resolve on the client, in the page's
    /// post-hydration `Effect`.
    pub fn resolve(&self, page: TimelinePage) {
        self.adopt(page);
        self.status.set(LoadStatus::Idle);
    }

    /// Record a fetch failure: empty the rows (don't show a stale page), clear
    /// the cursor + `has_more` so a failed timeline offers no "Load more", and
    /// mark the failure for display.
    pub fn fail(&self, message: String) {
        self.rows.set(Vec::new());
        self.cursor.set(None);
        self.has_more.set(false);
        self.status.set(LoadStatus::Failed(message));
    }
}

/// wasm-only load-more: fetch the next page with the current cursor and append
/// it. `fetch` is the page's list fn (`list_local_timeline` / `list_home_feed`).
pub fn spawn_load_more<F, Fut>(state: TimelineState, fetch: F)
where
    F: FnOnce(Option<UtcInstant>, Option<PostId>, Option<PageSize>) -> Fut + 'static,
    Fut: Future<Output = WebResult<TimelinePage>> + 'static,
{
    if state.status.get_untracked().is_in_flight() || !state.has_more.get_untracked() {
        return;
    }
    state.status.set(LoadStatus::InFlight);
    let (created_at, post_id) = TimelineCursor::into_query(state.cursor.get_untracked());
    spawn_local(async move {
        match fetch(created_at, post_id, Some(PageSize::default())).await {
            Ok(page) => {
                state.cursor.set(TimelineCursor::from_page(&page));
                state.has_more.set(page.has_more);
                state.rows.update(|rows| rows.extend(page.posts));
                state.status.set(LoadStatus::Idle);
            }
            Err(err) => state.status.set(LoadStatus::Failed(err.to_string())),
        }
    });
}

/// The scroll region shared by both timelines: the post list (or an empty
/// placeholder) followed by the load-more button.
#[component]
pub fn TimelineRows(
    state: TimelineState,
    on_mutate: Callback<()>,
    on_load_more: Callback<()>,
) -> impl IntoView {
    let read_rows = move || read_signal!(state.rows);
    let read_has_more = move || read_signal!(state.has_more);
    let read_in_flight = move || read_signal!(state.status).is_in_flight();
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
                            <button on:click=move |_| on_load_more.run(()) disabled=read_in_flight>
                                {move || {
                                    if read_in_flight() { "Loading\u{2026}" } else { "Load more" }
                                }}
                            </button>
                        }
                    })
            }}
        </div>
    }
}

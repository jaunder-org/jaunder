//! Timeline pagination — the wasm-only layer (ADR-0070): the load-more task and
//! the shared `TimelineRows` view. The value model *and* the reactive
//! `TimelineState` signal bundle live in the ungated, host-tested `state.rs`
//! (#671); what stays here is only what cannot run on the host — `spawn_local`
//! and the `view!` tree. This file carries no cfg gates of its own (its `mod`
//! declaration is `#[cfg(target_arch = "wasm32")]`).

use std::future::Future;

use leptos::prelude::*;
use leptos::task::spawn_local;

use common::ids::PostId;
use common::pagination::PageSize;
use common::seed::TimelinePage;
use common::time::UtcInstant;

use super::state::{LoadStatus, TimelineCursor, TimelineState};
use crate::error::WebResult;
use crate::posts::PostCard;
use crate::taglist::TagCtx as TagContext;

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
            Err(err) => state.status.set(LoadStatus::Failed(err)),
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
    /// Tag-chip linking context for each row's `PostCard`. Defaults to
    /// `SiteWide` (the site/cockpit timelines); the user timeline passes
    /// `ForUser` so chips also render the "· here" per-author link.
    #[prop(default = TagContext::SiteWide)]
    tag_context: TagContext,
    /// Empty-state message when there are no rows. Defaults to the generic
    /// "No posts yet."; the tag pages pass "No posts with this tag yet.".
    #[prop(default = "No posts yet.")]
    empty_text: &'static str,
) -> impl IntoView {
    let read_rows = move || state.rows.get();
    let read_has_more = move || state.has_more.get();
    let read_in_flight = move || state.status.get().is_in_flight();
    view! {
        <div class="j-scroll">
            {move || {
                let rows = read_rows();
                if rows.is_empty() {
                    view! { <p>{empty_text}</p> }.into_any()
                } else {
                    rows.into_iter()
                        .map(|p| {
                            view! {
                                <PostCard
                                    post=p
                                    banner=None
                                    tag_context=tag_context.clone()
                                    on_mutate=on_mutate
                                />
                            }
                        })
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

//! Timeline pagination — the wasm-only layer (ADR-0070): the load-more task, the
//! resolve wiring, and the shared `TimelineRows` / `TimelineGate` views. The value
//! model *and* the reactive `TimelineState` signal bundle live in the ungated,
//! host-tested `state.rs` (#671); what stays here is only what cannot run on the
//! host — `Effect::new`, `spawn_local`, and the `view!` trees. This file carries no
//! cfg gates of its own (its `mod` declaration is `#[cfg(target_arch = "wasm32")]`).

use std::future::Future;

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::Redirect;

use common::pagination::PageSize;
use common::seed::{PageCursor, TimelinePage};

use super::state::{NoIdentity, TimelinePaint, TimelineState};
use crate::error::WebResult;
use crate::posts::PostCard;
use crate::taglist::TagCtx as TagContext;

/// wasm-only load-more: fetch the next page with the current cursor and append
/// it. `fetch` is the page's list fn (`list_local_timeline` / `list_home_feed`).
pub fn spawn_load_more<F, Fut>(state: TimelineState, fetch: F)
where
    F: FnOnce(Option<PageCursor>, Option<PageSize>) -> Fut + 'static,
    Fut: Future<Output = WebResult<TimelinePage>> + 'static,
{
    // The guard, the cursor read, and the result fold are all host-tested on
    // `TimelineState` (#671); what cannot run on the host — and so all that is left
    // here — is `spawn_local`.
    let Some(claim) = state.begin_load_more() else {
        return;
    };
    spawn_local(async move {
        state.append(fetch(claim.cursor, Some(PageSize::default())).await);
    });
}

/// Wire a page's initial-fetch `Resource` into the shared timeline state: the one
/// canonical CSR resolve, replacing the identical `Effect` four pages carried.
///
/// The `Effect` itself is irreducibly wasm-only — it does not run in a host test
/// (`web::reactive`) — but everything it decides lives in the host-tested
/// `TimelineState::apply`.
pub fn wire_timeline_resolve(
    state: TimelineState,
    initial_page: Resource<WebResult<TimelinePage>>,
) {
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            state.apply(result);
        }
    });
}

/// The shared error → loading → rows gate every timeline page paints through.
///
/// The body is a `Memo` plus a bare `match`: the decision itself is
/// [`TimelineState::paint`], host-tested in `state.rs`, so nothing branchy hides in
/// this wasm-only view (#671, #306).
#[component]
pub fn TimelineGate(
    state: TimelineState,
    on_mutate: Callback<()>,
    on_load_more: Callback<()>,
    /// Row context for each `PostCard`'s tag chips, and the page's route-derived
    /// identity in one: `None` means the URL segment has not resolved to a user, so
    /// no rows are painted. Defaults to site-wide, which four of five pages want.
    #[prop(default = Signal::derive(|| Some(TagContext::SiteWide)))]
    tag_context: Signal<Option<TagContext>>,
    /// Empty-state message when there are no rows. Defaults to the generic
    /// "No posts yet."; the tag pages pass "No posts with this tag yet.".
    #[prop(default = "No posts yet.")]
    empty_text: &'static str,
    /// What to paint when there is no identity to show a timeline for. The cockpit
    /// passes `Redirect("/login")`; everyone else renders nothing.
    #[prop(default = NoIdentity::Blank)]
    no_identity: NoIdentity,
    /// Page chrome that accompanies the timeline — `home`'s masthead, the cockpit's
    /// topbar + composer. Rendered in the loading and rows arms only, never over an
    /// error or a redirect.
    #[prop(optional)]
    children: Option<ChildrenFn>,
) -> impl IntoView {
    // A `Memo`, not a bare closure: `status` is written on every refetch (→ Idle)
    // and every load-more (→ InFlight → Idle). Reading it raw here would re-run the
    // match on each of those writes and REMOUNT `TimelineRows`, rebuilding every
    // `PostCard` on every paginate. The memo dedupes, so only a real transition
    // re-paints.
    let paint = Memo::new(move |_| state.paint(tag_context.get()));
    // Chrome is its own sibling region rather than `{children}` inside each arm:
    // emitting it per-arm would tear the subtree down and rebuild it on every
    // `Loading → Rows`. For `home` that subtree is the `inner_html` masthead —
    // projector-coincident markup (ADR-0041 §2), the class of bug #653 was. This
    // memo dedupes `true → true`, so for home the chrome is built once and survives
    // the transition. (The cockpit's children read `username`, which flips at the
    // same moment, so its subtree is rebuilt regardless — as it is today.)
    let show_chrome = Memo::new(move |_| paint.get().is_ok_and(|paint| paint.shows_chrome()));
    view! {
        {move || show_chrome.get().then(|| children.clone().map(|children| children()))}
        {move || match paint.get() {
            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
            Ok(TimelinePaint::Loading) => {
                view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any()
            }
            Ok(TimelinePaint::Rows(tag_context)) => {
                view! {
                    <TimelineRows
                        state=state
                        on_mutate=on_mutate
                        on_load_more=on_load_more
                        tag_context=tag_context
                        empty_text=empty_text
                    />
                }
                    .into_any()
            }
            Ok(TimelinePaint::Unidentified) => {
                match no_identity {
                    NoIdentity::Blank => ().into_any(),
                    NoIdentity::Redirect(path) => view! { <Redirect path=path /> }.into_any(),
                }
            }
        }}
    }
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

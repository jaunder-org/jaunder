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
use wasm_bindgen::JsCast;

use common::pagination::PageSize;
use common::seed::{Page, RenderedPost, TimelineCursor, TimelineOrder};

use super::state::{NoIdentity, TimelinePaint, TimelineState};
use crate::error::WebResult;
use crate::posts::PostCard;
use crate::taglist::TagCtx;

/// wasm-only load-more: fetch the next page with the current cursor and append
/// it. `fetch` is the page's list fn (`list_local_timeline` / `list_home_feed`).
pub fn spawn_load_more<F, Fut>(state: TimelineState, fetch: F)
where
    F: FnOnce(Option<TimelineCursor>, Option<PageSize>) -> Fut + 'static,
    Fut: Future<Output = WebResult<Page<RenderedPost, TimelineCursor>>> + 'static,
{
    // The guard, the cursor read, and the result fold are all host-tested on
    // `TimelineState` (#671); what cannot run on the host — and so all that is left
    // here — is `spawn_local`.
    let Some(claim) = state.begin_load_more() else {
        return;
    };
    spawn_local(async move {
        state.append(claim, fetch(claim.cursor, Some(PageSize::default())).await);
    });
}

/// Adopt a route presentation and page only after the current resource resolves
/// and its custom stylesheet becomes active.
///
/// `Resource` publishes only the latest keyed destination; observing its pending
/// state also cancels any staged stylesheet as soon as navigation supersedes it.
pub fn wire_timeline_destination(
    state: TimelineState,
    destination: Resource<
        WebResult<(
            common::theme::PublishedThemePresentation,
            Page<RenderedPost, TimelineCursor>,
        )>,
    >,
    presentation: crate::app::ThemePresentationCoordinator,
) {
    Effect::new(move |_| match destination.try_get().flatten() {
        Some(Ok((theme, page))) => {
            spawn_local(async move {
                match presentation.adopt(theme).await {
                    Ok(crate::app::ThemeAdoption::Applied) => state.apply(Ok(page)),
                    Ok(crate::app::ThemeAdoption::Superseded) => {}
                    Err(error) => state.apply(Err(error)),
                }
            });
        }
        Some(Err(error)) => state.apply(Err(error)),
        None => presentation.begin_navigation(),
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
    order: Signal<TimelineOrder>,
    on_order_change: Callback<TimelineOrder>,
    /// Row context for each `PostCard`'s tag chips, and the page's route-derived
    /// identity in one: `None` means the URL segment has not resolved to a user, so
    /// no rows are painted. Defaults to site-wide, which four of five pages want.
    #[prop(default = Signal::derive(|| Some(TagCtx::SiteWide)))]
    tag_context: Signal<Option<TagCtx>>,
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
                        order=order
                        on_order_change=on_order_change
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
    order: Signal<TimelineOrder>,
    on_order_change: Callback<TimelineOrder>,
    /// Tag-chip linking context for each row's `PostCard`. Defaults to
    /// `SiteWide` (the site/cockpit timelines); the user timeline passes
    /// `ForUser` so chips also render the "· here" per-author link.
    #[prop(default = TagCtx::SiteWide)]
    tag_context: TagCtx,
    /// Empty-state message when there are no rows. Defaults to the generic
    /// "No posts yet."; the tag pages pass "No posts with this tag yet.".
    #[prop(default = "No posts yet.")]
    empty_text: &'static str,
) -> impl IntoView {
    let read_rows = move || state.rows.get();
    let read_has_more = move || state.has_more.get();
    let read_in_flight = move || state.status.get().is_in_flight();
    let toggle_order = move |event: web_sys::MouseEvent| {
        let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        let Ok(Some(_)) = target.closest("[data-jaunder-part=\"timeline-order\"]") else {
            return;
        };
        on_order_change.run(super::render::opposite_order(order.get_untracked()));
    };
    view! {
        <div class="j-scroll" on:click=toggle_order>
            {move || {
                super::render::order_control(order.get())
                    .inject_into(leptos::html::div().class("j-contents"))
            }}
            <div data-jaunder-part="post-list">
                {move || {
                    let rows = read_rows();
                    if rows.is_empty() {
                        view! { <p>{empty_text}</p> }.into_any()
                    } else {
                        rows.iter()
                            .map(|p| {
                                view! {
                                    <PostCard
                                        post=p
                                        banner=None
                                        tag_context=&tag_context
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
                                <button
                                    data-jaunder-part="continuation"
                                    on:click=move |_| on_load_more.run(())
                                    disabled=read_in_flight
                                >
                                    {move || {
                                        if read_in_flight() {
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
        </div>
    }
}

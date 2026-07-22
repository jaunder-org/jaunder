//! The home vertical's wasm-only UI (ADR-0070): the routed `/` public
//! Local-timeline landing page. Renders the shared `crate::render` masthead via
//! `inner_html` (coincidence with the projector, ADR-0041) + the reactive
//! `crate::timeline` rows. No cfgs of its own (wasm-only via its `mod` line).

use leptos::prelude::*;

use crate::feed_discovery::FeedDiscovery;
use crate::pages::signal_read::read_signal;
use crate::posts::list_local_timeline;
use crate::timeline::{TimelineRows, TimelineState};
use common::feed::FeedSurface;
use common::pagination::PageSize;

#[component]
pub fn HomePage() -> impl IntoView {
    let state = TimelineState::default();

    // Public projector seed (#178/#179): `/` is the anonymous site (Local) timeline
    // for EVERYONE, including the authenticated owner — the owner stays on this
    // enhanced public front page (#181, ADR-0044 D10) rather than swapping to a
    // personalized feed (a content swap can't be flash-free; the projector paints
    // anonymous-only bytes). The personalized Feed lives at the `/app` cockpit.
    // Adopt the seed as the initial state so first paint shows content, no swap.
    if let Some(crate::render::PageSeed::SiteTimeline(page)) =
        leptos::prelude::use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        state.adopt(page);
    }

    let refresh_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| refresh_version.update(|v| *v += 1));

    // The Local timeline is identical for every viewer, so the fetch is
    // viewer-independent — no `current_user()` gate and no mode swap (#181, D10).
    // Re-fetch on mutation (`refresh_version`) so the owner's own edits/deletes,
    // performed via the client-side action column, reflect immediately.
    let initial_page = Resource::new(
        move || refresh_version.get(),
        |_| list_local_timeline(None, None, Some(PageSize::default())),
    );

    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(page) => state.resolve(page),
                Err(err) => state.fail(err.to_string()),
            }
        }
    });

    let on_load_more = Callback::new(move |()| {
        crate::timeline::spawn_load_more(state, list_local_timeline);
    });

    // A `Memo` (not a bare closure) so the outer view closure re-runs only when
    // the failure message changes — not on every `status` write (`resolve()` sets
    // `Idle` on each refresh; load-more toggles `InFlight`). Reading `status` raw
    // would needlessly rebuild the whole page on every refresh/paginate.
    let read_error = Memo::new(move |_| read_signal!(state.status).into_failure());

    // The masthead (topbar + anon Sign-in/Register links + hero) is the shared
    // pure fn the projector renders too, so both sides coincide by construction
    // (ADR-0041 §2) — no `view!` twin to drift. The anon-only CTA lives inside it,
    // hidden for the authed owner via `j-anon-only` + `html.authed` (ADR-0044),
    // and shown for the anonymous visitor. Single-mode Local (#181, D10): `/` is
    // always the enhanced public timeline; the owner's own posts gain the
    // client-side action column reactively via `TimelineRows`/`PostCard`.
    let masthead = crate::render::render_home_masthead();

    view! {
        <FeedDiscovery surface=FeedSurface::Site />
        {move || {
            if let Some(err) = read_error.get() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            view! {
                <div style="display:contents" inner_html=masthead.clone()></div>
                <TimelineRows state=state on_mutate=on_mutate on_load_more=on_load_more />
            }
                .into_any()
        }}
    }
}

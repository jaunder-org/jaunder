//! The home vertical's wasm-only UI (ADR-0070): the routed `/` public
//! Local-timeline landing page. Renders the co-located `crate::home::render` masthead via
//! `inner_html` (coincidence with the projector, ADR-0041) + the reactive
//! `crate::timeline` rows. No cfgs of its own (wasm-only via its `mod` line).

use leptos::prelude::*;

use crate::feed_discovery::FeedDiscovery;
use crate::reactive::Invalidator;
use crate::timeline::{self, TimelineGate, TimelineState};
use common::seed::PageSeed;
use common::{feed::FeedSurface, pagination::PageSize};

#[component]
pub fn HomePage() -> impl IntoView {
    let state = TimelineState::default();

    // Public projector seed (#178/#179): `/` is the anonymous site (Local) timeline
    // for EVERYONE, including the authenticated owner — the owner stays on this
    // enhanced public front page (#181, ADR-0044 D10) rather than swapping to a
    // personalized feed (a content swap can't be flash-free; the projector paints
    // anonymous-only bytes). The personalized Feed lives at the `/app` cockpit.
    // Adopt the seed as the initial state so first paint shows content, no swap.
    // No URL guard is needed (unlike the tag/profile pages): the `SiteTimeline`
    // variant itself identifies `/`, so a seed carried over from another route
    // cannot match.
    state.adopt_seed(
        match leptos::prelude::use_context::<Option<PageSeed>>().flatten() {
            Some(PageSeed::SiteTimeline(page)) => Some(page),
            _ => None,
        },
    );

    let invalidator = Invalidator::new();
    let on_mutate = Callback::new(move |()| invalidator.notify());

    // The Local timeline is identical for every viewer, so the fetch is
    // viewer-independent — no `current_user()` gate and no mode swap (#181, D10).
    // Re-fetch after a committed mutation so the owner's own edits/deletes,
    // performed via the client-side action column, reflect immediately.
    let initial_page = client::reactive::resource(
        move || invalidator.track(),
        move || timeline::list_local_timeline(None, Some(PageSize::default())),
    );

    timeline::wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        timeline::spawn_load_more(state, timeline::list_local_timeline);
    });

    // The masthead (topbar + anon Sign-in/Register links + hero) is the shared
    // pure fn the projector renders too, so both sides coincide by construction
    // (ADR-0041 §2) — no `view!` twin to drift. The anon-only CTA lives inside it,
    // hidden for the authed owner via `j-anon-only` + `html.authed` (ADR-0044),
    // and shown for the anonymous visitor. Single-mode Local (#181, D10): `/` is
    // always the enhanced public timeline; the owner's own posts gain the
    // client-side action column reactively via `TimelineRows`/`PostCard`.
    let masthead = super::render::render_masthead();

    view! {
        <FeedDiscovery surface=&FeedSurface::Site />
        // loading and rows arms but not over an error — the error branch replaces
        // masthead + rows together. The gate keeps that subtree alive across
        // `Loading → Rows` rather than rebuilding it, which matters here because it
        // is projector-coincident markup (ADR-0041 §2).
        <TimelineGate state=state on_mutate=on_mutate on_load_more=on_load_more>
            {masthead.clone().inject_into(leptos::html::div().style("display:contents"))}
        </TimelineGate>
    }
}

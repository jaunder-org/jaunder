//! The `/app` cockpit (#181, ADR-0044 D6): the authenticated owner's personalized
//! home Feed, relocated off `/` (which stays the enhanced public timeline, D10). A
//! first-class, directly-bookmarkable authed-only route — served from the SPA
//! shell (`no-store`), pre-painted `html.authed`, so a direct hit boots straight
//! into the feed with zero clicks. An anonymous / expired visitor bounces to
//! `/login`.

use common::pagination::PageSize;
use leptos::prelude::*;

use super::CockpitState;
use crate::posts::InlineComposer;
use crate::reactive::Invalidator;
use crate::timeline::{self, NoIdentity, TimelineGate};
use crate::topbar::Topbar;

#[component]
pub fn CockpitPage() -> impl IntoView {
    // The signal bundle and every transition below are host-tested in `super::state`
    // (#306, ADR-0083); this body keeps only the `Effect` and the `view!`.
    let state = CockpitState::default();

    let invalidator = Invalidator::new();
    let on_mutate = Callback::new(move |()| invalidator.notify());

    // Gate on the shared session's server-confirmed reconcile, then fetch the
    // personalized feed. Unlike `/`, `/app` is authed-only and served from the SPA
    // shell (no-store), so an async gate is correct here — there is no cacheable-page
    // flash constraint. `Ok(None)` means anonymous / expired → bounce to `/login`
    // (D6). Publish, delete, and unpublish settlements notify the page invalidator;
    // the reconcile itself is keyed on pathname, so this reuses it rather than
    // re-hitting the server for identity on every publish (#591).
    let session = crate::auth::use_session();
    let initial_page = client::reactive::resource(
        move || invalidator.track(),
        move || async move {
            super::resolve_initial_page(session.reconcile.await, || {
                timeline::list_home_feed(None, Some(PageSize::default()))
            })
            .await
        },
    );

    // Copy the resolved Resource into the page's signals once it loads. This `Effect`
    // stays page-specific (#671): the payload carries the session-confirmed identity,
    // which no shared helper can publish. Every transition it dispatches to —
    // including the anonymous/expired bounce (D6), which travels on the timeline
    // status rather than a separate `bounce` signal — is host-tested in
    // `super::state`.
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            state.apply(result);
        }
    });

    let on_load_more = Callback::new(move |()| {
        timeline::spawn_load_more(state.timeline, timeline::list_home_feed);
    });

    let read_username = move || state.username.get();

    view! {
        // Only the CHROME goes in `children` — the gate itself owns the loading
        // placeholder and the rows. The chrome renders in the loading and rows arms
        // but not over the error banner or the redirect: four outcomes, chrome on
        // exactly two.
        <TimelineGate
            state=state.timeline
            on_mutate=on_mutate
            on_load_more=on_load_more
            no_identity=NoIdentity::Redirect("/login")
        >
            {move || match read_username() {
                None => view! { <Topbar title="Home" /> }.into_any(),
                Some(user) => {
                    view! {
                        <Topbar title="Home" sub="Your home feed" />
                        <InlineComposer username=user on_publish=on_mutate />
                    }
                        .into_any()
                }
            }}
        </TimelineGate>
    }
}

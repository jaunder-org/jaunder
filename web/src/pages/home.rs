use leptos::prelude::*;

use crate::feed_discovery::FeedDiscovery;
use crate::pages::signal_read::read_signal;
use crate::pages::timeline::{TimelineRows, TimelineState};
use crate::pages::ui::Topbar;
use crate::posts::list_local_timeline;
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
    let initial_page = crate::server_resource(
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
        crate::pages::timeline::spawn_load_more(state, list_local_timeline);
    });

    let read_error = move || read_signal!(state.error);

    view! {
        <FeedDiscovery surface=FeedSurface::Site />
        {move || {
            if let Some(err) = read_error() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            // Single-mode Local (#181, D10): `/` is always the enhanced public
            // timeline. The owner's own posts gain the client-side action column
            // (see `PostCard`'s marker match); the anon-only Sign in / Register CTA
            // is hidden for the owner via `html.authed` CSS (pre-paint, flash-free).
            view! {
                <Topbar
                    title="jaunder.local"
                    sub="Read-only \u{00b7} posts originating on this instance"
                >
                    <a href="/login" class="j-btn j-anon-only">
                        "Sign in"
                    </a>
                    <a href="/register" class="j-btn is-primary j-anon-only">
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
                <TimelineRows state=state on_mutate=on_mutate on_load_more=on_load_more />
            }
                .into_any()
        }}
    }
}

//! The `/app` cockpit (#181, ADR-0044 D6): the authenticated owner's personalized
//! home Feed, relocated off `/` (which stays the enhanced public timeline, D10). A
//! first-class, directly-bookmarkable authed-only route — served from the SPA
//! shell (`no-store`), pre-painted `html.authed`, so a direct hit boots straight
//! into the feed with zero clicks. An anonymous / expired visitor bounces to
//! `/login`. This is the former `home.rs` Feed branch moved to its proper home.

use common::pagination::PageSize;
use common::username::Username;
use leptos::prelude::*;

use crate::posts::{list_home_feed, InlineComposer};
use crate::timeline::{NoIdentity, TimelineGate, TimelineState};
use crate::topbar::Topbar;

#[component]
pub fn CockpitPage() -> impl IntoView {
    let state = TimelineState::default();
    let username = RwSignal::new(None::<Username>);

    let refresh_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| refresh_version.update(|v| *v += 1));

    // Gate on the shared session's server-confirmed reconcile, then fetch the
    // personalized feed. Unlike `/`, `/app` is authed-only and served from the SPA
    // shell (no-store), so an async gate is correct here — there is no cacheable-page
    // flash constraint. `Ok(None)` means anonymous / expired → bounce to `/login`
    // (D6). Keyed on `refresh_version` (publish/draft), which refetches the feed; the
    // reconcile itself is keyed on pathname, so this reuses it rather than re-hitting
    // the server for identity on every publish (#591).
    let session = crate::auth::use_session();
    let initial_page = Resource::new(
        move || refresh_version.get(),
        move |_| async move {
            match session.reconcile.await {
                Ok(Some(user)) => list_home_feed(None, None, Some(PageSize::default()))
                    .await
                    .map(|page| Some((user.username, page))),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        },
    );

    // Copy the resolved Resource into the timeline signals once it loads. This
    // `Effect` stays page-specific (#671): the payload carries the session-confirmed
    // identity, which no shared helper can publish. Only the *transitions* it
    // dispatches to are shared, and those are host-tested.
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(Some((user, page))) => {
                    // Only set `username` when it actually changes: a spurious set
                    // would re-run the chrome closure and REMOUNT InlineComposer,
                    // wiping its publish/draft flash (a re-fetch fires on every
                    // publish via `refresh_version`).
                    if username.get_untracked().as_ref() != Some(&user) {
                        username.set(Some(user));
                    }
                    state.adopt(page);
                }
                // Anonymous / expired (D6). The status carries the bounce now — no
                // separate `bounce` signal — and the gate's `no_identity` prop turns
                // it into the `/login` redirect.
                Ok(None) => state.unidentified(),
                Err(err) => state.fail(err),
            }
        }
    });

    let on_load_more = Callback::new(move |()| {
        crate::timeline::spawn_load_more(state, list_home_feed);
    });

    let read_username = move || username.get();

    view! {
        // Only the CHROME goes in `children` — the gate itself owns the loading
        // placeholder and the rows. It renders in the loading and rows arms but not
        // over the error banner or the redirect, which reproduces all four of this
        // page's previous outcomes exactly.
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            no_identity=NoIdentity::Redirect("/login")
        >
            {move || match read_username() {
                None => view! { <Topbar title="Home" /> }.into_any(),
                Some(user) => {
                    view! {
                        <Topbar title="Home" sub="Your home feed" />
                        <InlineComposer username=user on_publish=refresh_version.write_only() />
                    }
                        .into_any()
                }
            }}
        </TimelineGate>
    }
}

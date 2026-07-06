//! The `/app` cockpit (#181, ADR-0044 D6): the authenticated owner's personalized
//! home Feed, relocated off `/` (which stays the enhanced public timeline, D10). A
//! first-class, directly-bookmarkable authed-only route — served from the SPA
//! shell (`no-store`), pre-painted `html.authed`, so a direct hit boots straight
//! into the feed with zero clicks. An anonymous / expired visitor bounces to
//! `/login`. This is the former `home.rs` Feed branch moved to its proper home.

use leptos::prelude::*;
use leptos_router::components::Redirect;

use crate::auth::current_user;
use crate::pages::signal_read::read_signal;
use crate::pages::timeline::{TimelineRows, TimelineState, PAGE_SIZE};
use crate::pages::ui::{InlineComposer, Topbar};
use crate::posts::list_home_feed;

#[component]
pub fn CockpitPage() -> impl IntoView {
    let state = TimelineState::default();
    let username = RwSignal::new(None::<String>);
    let bounce = RwSignal::new(false);

    let refresh_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| refresh_version.update(|v| *v += 1));

    // Gate on `current_user`, then fetch the personalized feed. Unlike `/`, `/app`
    // is authed-only and served from the SPA shell (no-store), so an async gate is
    // correct here — there is no cacheable-page flash constraint. `Ok(None)` means
    // anonymous / expired → bounce to `/login` (D6).
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    let initial_page = crate::server_resource(
        move || refresh_version.get(),
        |_| async move {
            match current_user().await {
                Ok(Some(user)) => list_home_feed(None, None, Some(PAGE_SIZE))
                    .await
                    .map(|page| Some((user, page))),
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            }
        },
    );

    // Client-only effect copies the resolved Resource into the timeline signals.
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(Some((user, page))) => {
                    // Only set `username` when it actually changes: a spurious set
                    // would re-run the outer view closure and REMOUNT InlineComposer,
                    // wiping its publish/draft flash (a re-fetch fires on every
                    // publish via `refresh_version`).
                    if username.get_untracked().as_ref() != Some(&user) {
                        username.set(Some(user));
                    }
                    state.resolve(page);
                }
                Ok(None) => bounce.set(true),
                Err(err) => state.fail(err.to_string()),
            }
        }
    });

    let on_load_more = Callback::new(move |()| {
        #[cfg(target_arch = "wasm32")]
        crate::pages::timeline::spawn_load_more(state, |c1, c2, n| list_home_feed(c1, c2, n));
    });

    let read_error = move || read_signal!(state.error);
    let read_bounce = move || read_signal!(bounce);
    let read_username = move || read_signal!(username);

    view! {
        {move || {
            if read_bounce() {
                return view! { <Redirect path="/login" /> }.into_any();
            }
            if let Some(err) = read_error() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            match read_username() {
                None => {
                    view! {
                        <Topbar title="Home".to_string() />
                        <p class="j-loading">"Loading\u{2026}"</p>
                    }
                        .into_any()
                }
                Some(user) => {
                    view! {
                        <Topbar title="Home".to_string() sub="Your home feed".to_string() />
                        <InlineComposer username=user on_publish=refresh_version.write_only() />
                        <TimelineRows state=state on_mutate=on_mutate on_load_more=on_load_more />
                    }
                        .into_any()
                }
            }
        }}
    }
}

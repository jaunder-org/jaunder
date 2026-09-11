use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_params_map, use_query_map};

use crate::feed_discovery::{FeedDiscovery, RsdDiscovery};
use crate::posts::ListingRoute;
use crate::reactive::Invalidator;
use crate::subscriptions::SubscribeButton;
use crate::timeline::{self, TimelineGate, TimelineState};
use crate::topbar::Topbar;
use common::seed::PageSeed;
use common::tag::Tag;
use common::username::Username;

/// The one concrete lifecycle for every public Post listing route. The route owns
/// validation, endpoint choice, exact seed adoption, and chrome data; this wasm
/// layer owns only resources, effects, spawning, and view construction.
#[component]
fn PublicListingPage(route: Memo<ListingRoute>) -> impl IntoView {
    let state = TimelineState::default();
    let invalidator = Invalidator::new();
    let presentation = crate::app::theme_presentation();
    let theme = crate::app::public_theme();
    let seed = use_context::<Option<PageSeed>>().flatten();

    state.adopt_seed(route.get_untracked().seeded_page(seed));
    let first_destination = AtomicBool::new(true);

    let destination = Resource::new(
        move || {
            if !first_destination.swap(false, Ordering::Relaxed) {
                state.begin_replacement();
            }
            (route.get(), invalidator.track())
        },
        move |(route, _)| route.destination(),
    );
    timeline::wire_timeline_destination(state, destination, presentation);

    let on_mutate = Callback::new(move |()| invalidator.notify());
    let on_load_more = Callback::new(move |()| {
        let route = route.get_untracked();
        timeline::spawn_load_more(state, move |cursor, limit| async move {
            Ok(route.fetch_page(cursor, limit).await?.page)
        });
    });
    let navigate = use_navigate();
    let on_order_change = Callback::new(move |order| {
        let base = route.get_untracked().timeline_base_url();
        navigate(
            &timeline::order_url(&base, order),
            NavigateOptions::default(),
        );
    });
    let empty_text = route.get_untracked().empty_text();

    view! {
        {move || {
            route
                .get()
                .feed_surface()
                .map(|surface| {
                    view! { <FeedDiscovery surface=&surface /> }
                })
        }}
        {move || {
            route
                .get()
                .user_chrome()
                .map(|username| {
                    view! { <RsdDiscovery username=&username /> }
                })
        }}
        <Topbar title=move || route.get().title() sub=move || route.get().subtitle() />
        {move || {
            crate::app::render_theme_header(&theme.get())
                .inject_into(leptos::html::div().class("j-contents"))
        }}
        {move || {
            route
                .get()
                .user_chrome()
                .map(|username| {
                    view! { <SubscribeButton username=username /> }
                })
        }}
        <TimelineGate
            state
            on_mutate
            on_load_more
            order=Signal::derive(move || route.get().order())
            on_order_change
            tag_context=Signal::derive(move || route.get().tag_context())
            empty_text
        />
    }
}

#[component]
pub fn UserTimelinePage() -> impl IntoView {
    let params = use_params_map();
    let query = use_query_map();
    let route = Memo::new(move |_| {
        let order = ListingRoute::parse_order(query.get().get("order").as_deref());
        ListingRoute::Profile(
            params
                .get()
                .get("username")
                .unwrap_or_default()
                .strip_prefix('~')
                .and_then(|value| value.parse::<Username>().ok()),
            order,
        )
    });
    view! { <PublicListingPage route /> }
}

/// Site-wide listing of posts carrying a tag, at `/tags/:tag`.
#[component]
pub fn SiteTagPage() -> impl IntoView {
    let params = use_params_map();
    let query = use_query_map();
    let route = Memo::new(move |_| {
        let order = ListingRoute::parse_order(query.get().get("order").as_deref());
        ListingRoute::SiteTag(
            params
                .get()
                .get("tag")
                .and_then(|value| value.parse::<Tag>().ok()),
            order,
        )
    });
    view! { <PublicListingPage route /> }
}

/// Per-user listing of posts carrying a tag, at `/~:username/tags/:tag`.
#[component]
pub fn UserTagPage() -> impl IntoView {
    let params = use_params_map();
    let query = use_query_map();
    let route = Memo::new(move |_| {
        let params = params.get();
        let username = params
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .and_then(|value| value.parse::<Username>().ok());
        let tag = params
            .get("tag")
            .and_then(|value| value.parse::<Tag>().ok());
        let order = ListingRoute::parse_order(query.get().get("order").as_deref());
        ListingRoute::UserTag(username, tag, order)
    });
    view! { <PublicListingPage route /> }
}

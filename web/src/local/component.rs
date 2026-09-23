//! The Local vertical's wasm-only UI (ADR-0070): the routed `/` public landing page.
//! It renders the co-located `crate::local::render` masthead via `inner_html`
//! (coincidence with the projector, ADR-0041) plus reactive `crate::timeline` rows.
//! No cfgs of its own (wasm-only via its `mod` line).

use std::sync::atomic::{AtomicBool, Ordering};

use common::{feed::FeedSurface, pagination::PageSize, seed::TimelineOrder, site::SiteIdentity};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_meta::{Meta, Title};
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::feed_discovery::FeedDiscovery;
use crate::reactive::Invalidator;
use crate::timeline::{self, TimelineGate, TimelineState};

/// Commits a resolved Local destination only after its presentation is safe to paint.
/// The host-tested state fold lives in [`super::commit_destination`]; this wasm leaf
/// owns only the resource/effect and asynchronous theme-adoption wiring.
fn wire_local_destination(
    state: TimelineState,
    identity: RwSignal<Option<common::site::SiteIdentity>>,
    registration_policy: RwSignal<Option<common::registration::RegistrationPolicy>>,
    destination: Resource<crate::error::WebResult<super::LocalDestination>>,
    presentation: crate::app::ThemePresentationCoordinator,
) {
    Effect::new(move |_| match destination.try_get().flatten() {
        Some(Ok(destination)) => {
            spawn_local(async move {
                match presentation.adopt(destination.theme.clone()).await {
                    Ok(crate::app::ThemeAdoption::Applied) => {
                        super::commit_destination(
                            state,
                            identity,
                            registration_policy,
                            destination,
                        );
                    }
                    Ok(crate::app::ThemeAdoption::Superseded) => {}
                    Err(error) => state.fail(error),
                }
            });
        }
        Some(Err(error)) => state.fail(error),
        None => presentation.begin_navigation(),
    });
}

fn local_identity_metadata(identity: RwSignal<Option<SiteIdentity>>) -> impl IntoView {
    move || {
        identity.get().map(|identity| {
            view! {
                <Title text=identity.title.to_string() />
                <Meta
                    name="description"
                    content=identity.tagline.as_ref().map_or_else(String::new, ToString::to_string)
                />
                <Meta property="og:title" content=identity.title.to_string() />
                <Meta
                    property="og:description"
                    content=identity.tagline.as_ref().map_or_else(String::new, ToString::to_string)
                />
            }
        })
    }
}

/// Keep the projector-owned masthead bytes while wiring the sort action on its
/// containing node; inserting a reactive button here would duplicate it at mount.
fn interactive_masthead(
    markup: crate::html::Markup,
    order: Memo<TimelineOrder>,
    on_order_change: Callback<TimelineOrder>,
) -> impl IntoView {
    markup.inject_into(leptos::html::div().class("j-contents").on(
        leptos::ev::click,
        move |event| {
            timeline::handle_order_click(&event, order.get_untracked(), on_order_change);
        },
    ))
}

#[component]
pub fn LocalPage() -> impl IntoView {
    let presentation = crate::app::theme_presentation();
    let state = TimelineState::default();
    let query = use_query_map();
    let order = Memo::new(move |_| {
        query
            .get()
            .get("order")
            .and_then(|value| value.parse::<TimelineOrder>().ok())
            .unwrap_or_default()
    });

    // Public projector seed (#178/#179): `/` is the anonymous Local timeline. A
    // structurally valid auth marker redirects document loads to `/app` before this can
    // paint; a live session that lacks its marker may reach this recovery path once.
    // The projector still paints anonymous-only bytes, and Home remains the distinct
    // authenticated cockpit.
    let seed = super::site_timeline_seed(
        leptos::prelude::use_context::<Option<common::seed::PageSeed>>().flatten(),
    );
    // An unseeded Local route has no identity until its full destination is ready.
    // In particular, do not manufacture the historical "Jaunder" default while a
    // client-side navigation is still awaiting its theme.
    let identity = RwSignal::new(seed.identity);
    let registration_policy = RwSignal::new(seed.registration_policy);
    if seed.order == order.get_untracked() {
        state.adopt_seed(seed.page);
    }

    let invalidator = Invalidator::new();
    let on_mutate = Callback::new(move |()| invalidator.notify());

    // Local is identical for every viewer, so the fetch is viewer-independent — no
    // `current_user()` gate and no mode swap. Re-fetch after a committed mutation so
    // the owner's own edits/deletes, performed through the client-side Actions
    // disclosure, reflect immediately.
    let first_destination = AtomicBool::new(true);
    let initial_page = Resource::new(
        move || {
            if !first_destination.swap(false, Ordering::Relaxed) {
                state.begin_replacement();
            }
            (order.get(), invalidator.track())
        },
        move |(order, _)| async move {
            timeline::list_local_timeline(common::seed::TimelinePageRequest {
                order,
                cursor: None,
                limit: Some(PageSize::default()),
            })
            .await
            .map(super::site_destination)
        },
    );
    wire_local_destination(
        state,
        identity,
        registration_policy,
        initial_page,
        presentation,
    );

    let on_load_more = Callback::new(move |()| {
        let order = order.get_untracked();
        timeline::spawn_load_more(state, move |cursor, limit| async move {
            timeline::list_local_timeline(common::seed::TimelinePageRequest {
                order,
                cursor,
                limit,
            })
            .await
            .map(super::site_destination)
            .map(|destination| destination.page)
        });
    });
    let navigate = use_navigate();
    let route_base = super::site_timeline_base_url();
    let on_order_change = Callback::new(move |order| {
        navigate(
            &timeline::order_url(&route_base, order),
            NavigateOptions::default(),
        );
    });

    // The policy-aware masthead is the shared pure fn the projector renders too,
    // so both sides coincide by construction
    // (ADR-0041 §2) — no `view!` twin to drift. The anonymous CTA lives inside it and
    // is hidden by `j-anon-only` + `html.authed` when the advisory marker is present.
    // Local remains anonymous projection; any owner affordance is a client-side
    // Actions disclosure in `TimelineRows`/`PostCard`.
    let theme = crate::app::public_theme();

    view! {
        {local_identity_metadata(identity)}
        <FeedDiscovery surface=&FeedSurface::Site />
        // loading and rows arms but not over an error — the error branch replaces
        // masthead + rows together. The gate keeps that subtree alive across
        // `Loading → Rows` rather than rebuilding it, which matters here because it
        // is projector-coincident markup (ADR-0041 §2).
        <TimelineGate state=state on_mutate=on_mutate on_load_more=on_load_more>
            {move || {
                identity
                    .get()
                    .map(|identity| {
                        interactive_masthead(
                            crate::app::render_theme_hero(
                                &super::render::masthead(
                                    &identity,
                                    registration_policy.get(),
                                    &crate::app::render_theme_logo(&theme.get()),
                                    order.get(),
                                ),
                                &crate::app::render_theme_header(&theme.get()),
                            ),
                            order,
                            on_order_change,
                        )
                    })
            }}
        </TimelineGate>
    }
}

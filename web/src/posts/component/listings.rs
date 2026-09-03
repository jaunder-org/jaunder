use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::feed_discovery::{FeedDiscovery, RsdDiscovery};
use crate::posts;
use crate::posts::ListingRoute;
use crate::subscriptions::SubscribeButton;
use crate::taglist::TagCtx;
use crate::timeline::{self, TimelineGate, TimelineState};
use crate::topbar::Topbar;
use common::feed::FeedSurface;
use common::pagination::PageSize;
use common::seed::PageSeed;
use common::tag::Tag;
use common::username::Username;

fn canonical_username_display(username: Memo<Option<Username>>) -> impl Fn() -> String {
    move || username.get().map(String::from).unwrap_or_default()
}

#[component]
pub fn UserTimelinePage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `~username` route segment into `Username` once, at the source; an
    // invalid segment is `None` and every consumer handles the absence.
    let username = Memo::new(move |_| {
        params
            .get()
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .and_then(|s| s.parse::<Username>().ok())
    });

    let mutate_version = RwSignal::new(0u32);
    let theme = crate::app::public_theme();
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = Resource::new(
        move || (username.get(), mutate_version.get()),
        move |(username, _)| async move {
            posts::user_destination(username, |username| {
                timeline::list_by_user(username, None, Some(PageSize::default()))
            })
            .await
            .map(|(destination_theme, page)| {
                theme.set(destination_theme);
                page
            })
        },
    );

    let state = TimelineState::default();
    // Public projector seed (#178/#179): if the server painted this profile,
    // adopt its posts as the initial state so first paint shows content (no
    // Loading flash). The route guard — which keeps a client-side navigation to a
    // *different* profile from adopting the initial URL's seed — is the host-tested
    // `seeded_page` (#306); the reactive fetch still runs and takes over.
    state.adopt_seed(posts::seeded_page(
        use_context::<Option<PageSeed>>().flatten(),
        &ListingRoute::Profile(username.get_untracked()),
    ));

    timeline::wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        if let Ok(username) = posts::user_query(username.get_untracked()) {
            timeline::spawn_load_more(state, move |cursor, limit| async move {
                let presentation = timeline::list_by_user(username, cursor, limit).await?;
                Ok(presentation.page)
            });
        }
    });

    let display_username = canonical_username_display(username);

    view! {
        {move || {
            username
                .get()
                .map(|username| {
                    let surface = FeedSurface::User {
                        username: username.clone(),
                    };
                    view! {
                        <FeedDiscovery surface=&surface />
                        <RsdDiscovery username=&username />
                    }
                })
        }}
        <Topbar title=move || format!("Posts by {}", display_username()) sub="User timeline" />
        {move || { username.get().map(|username| view! { <SubscribeButton username=username /> }) }}
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            tag_context=Signal::derive(move || username.get().map(TagCtx::ForUser))
        />
    }
}

/// Site-wide listing of posts carrying a tag, at `/tags/:tag`.
#[component]
pub fn SiteTagPage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `:tag` route segment into a canonical `Tag` once, at the source
    // (ADR-0063 §4); an unparseable segment is `None`, so the fetch below is
    // skipped and the client 404s — mirroring the `PostPage` slug parse.
    // `Tag::from_str` lowercases, so the heading and the projected render coincide.
    let tag = Memo::new(move |_| params.get().get("tag").and_then(|s| s.parse::<Tag>().ok()));

    let mutate_version = RwSignal::new(0u32);
    let theme = crate::app::public_theme();
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = Resource::new(
        move || (tag.get(), mutate_version.get()),
        move |(tag, _)| async move {
            posts::tag_destination(tag, |tag| {
                timeline::list_by_tag(tag, None, Some(PageSize::default()))
            })
            .await
            .map(|(destination_theme, page)| {
                theme.set(destination_theme);
                page
            })
        },
    );

    let state = TimelineState::default();
    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // tag so first paint shows content (the host-tested `seeded_page` guard keeps a
    // client-side nav to a different tag from adopting the initial URL's seed, #306);
    // the reactive fetch still runs.
    state.adopt_seed(posts::seeded_page(
        use_context::<Option<PageSeed>>().flatten(),
        &ListingRoute::SiteTag(tag.get_untracked()),
    ));

    timeline::wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        if let Ok(tag_value) = posts::tag_query(tag.get_untracked()) {
            timeline::spawn_load_more(state, move |cursor, limit| async move {
                let presentation = timeline::list_by_tag(tag_value, cursor, limit).await?;
                Ok(presentation.page)
            });
        }
    });

    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();

    view! {
        {move || {
            tag.get()
                .map(|tag| {
                    let surface = FeedSurface::SiteTag { tag };
                    view! { <FeedDiscovery surface=&surface /> }
                })
        }}
        <Topbar title=move || format!("#{}", read_tag()) sub="Posts on this instance" />
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            empty_text="No posts with this tag yet."
        />
    }
}

/// Per-user listing of posts carrying a tag, at `/~:username/tags/:tag`.
#[component]
pub fn UserTagPage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `~username` route segment into `Username` once, at the source; an
    // invalid segment is `None` and every consumer handles the absence.
    let username = Memo::new(move |_| {
        params
            .get()
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .and_then(|s| s.parse::<Username>().ok())
    });
    // Parse the `:tag` route segment into a canonical `Tag` once, at the source
    // (ADR-0063 §4); an unparseable segment is `None`, so the fetch below is
    // skipped and the client 404s — mirroring the `PostPage` slug parse.
    // `Tag::from_str` lowercases, so the heading and the projected render coincide.
    let tag = Memo::new(move |_| params.get().get("tag").and_then(|s| s.parse::<Tag>().ok()));

    let mutate_version = RwSignal::new(0u32);
    let theme = crate::app::public_theme();
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = Resource::new(
        move || (username.get(), tag.get(), mutate_version.get()),
        move |(username, tag, _)| async move {
            posts::user_tag_destination(username, tag, |username, tag| {
                timeline::list_by_user_and_tag(username, tag, None, Some(PageSize::default()))
            })
            .await
            .map(|(destination_theme, page)| {
                theme.set(destination_theme);
                page
            })
        },
    );

    let state = TimelineState::default();
    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // username+tag so first paint shows content; both halves of the match are the
    // host-tested `seeded_page` (#306), and the reactive fetch still runs.
    state.adopt_seed(posts::seeded_page(
        use_context::<Option<PageSeed>>().flatten(),
        &ListingRoute::UserTag(username.get_untracked(), tag.get_untracked()),
    ));

    timeline::wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        if let Ok((username_value, tag_value)) =
            posts::user_tag_query(username.get_untracked(), tag.get_untracked())
        {
            timeline::spawn_load_more(state, move |cursor, limit| async move {
                let presentation =
                    timeline::list_by_user_and_tag(username_value, tag_value, cursor, limit)
                        .await?;
                Ok(presentation.page)
            });
        }
    });

    let read_username = canonical_username_display(username);
    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();

    view! {
        {move || {
            username
                .get()
                .zip(tag.get())
                .map(|(username, tag)| {
                    let surface = FeedSurface::UserTag {
                        username,
                        tag,
                    };
                    view! { <FeedDiscovery surface=&surface /> }
                })
        }}
        <Topbar
            title=move || format!("#{}", read_tag())
            sub=move || format!("Posts by ~{}", read_username())
        />
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            tag_context=Signal::derive(move || username.get().map(TagCtx::ForUser))
            empty_text="No posts with this tag yet."
        />
    }
}

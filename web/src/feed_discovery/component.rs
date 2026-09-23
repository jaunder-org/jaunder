use common::{
    feed::{self, FeedFormat, FeedSurface},
    username::Username,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_meta::{Link, Title};
use leptos_router::hooks::use_location;

use crate::error::WebResult;
use common::seed::{PageSeed, TimelinePageRequest};

use super::labels;
/// Renders feed auto-discovery link tags for RSS, Atom, and JSON Feed.
/// The component itself is invisible; it hoists `<link>` tags into the document head.
#[component]
pub fn FeedDiscovery<'a>(surface: &'a FeedSurface) -> impl IntoView + use<> {
    let label = labels::surface_label(surface);

    view! {
        <Link
            rel="alternate"
            type_="application/rss+xml"
            title=format!("{label} (RSS)")
            href=feed::canonicalize(surface, FeedFormat::Rss)
        />
        <Link
            rel="alternate"
            type_="application/atom+xml"
            title=format!("{label} (Atom)")
            href=feed::canonicalize(surface, FeedFormat::Atom)
        />
        <Link
            rel="alternate"
            type_="application/feed+json"
            title=format!("{label} (JSON Feed)")
            href=feed::canonicalize(surface, FeedFormat::Json)
        />
    }
}

/// The last User-tag timeline whose first page was actually resolved. A soft
/// shell fallback never claims a marker just because the path parses.
#[derive(Clone, Copy)]
pub struct ConfirmedUserTag(pub RwSignal<Option<FeedSurface>>);

impl ConfirmedUserTag {
    /// Missing shell context fails closed rather than guessing User existence.
    #[must_use]
    pub fn current() -> Self {
        use_context::<Self>().unwrap_or_else(|| Self(RwSignal::new(None)))
    }
}

/// A public index whose first paint adopts the exact projector seed. In-app
/// navigation resolves the same timeline presentation for theme/missing-context
/// policy, then paints through the projector's non-reactive markup builder.
#[component]
pub fn FeedIndexPage() -> impl IntoView {
    let location = use_location();
    let theme = crate::app::public_theme();
    let presentation = crate::app::theme_presentation();
    let seed = use_context::<Option<PageSeed>>().flatten();
    let path = location.pathname.get_untracked();
    let initial = super::routes::seeded_discovery(seed, &path);
    let surface = RwSignal::new(initial);
    let visible = Memo::new(move |_| {
        super::routes::visible_discovery_surface(surface.get(), &location.pathname.get())
    });
    let error = RwSignal::<Option<String>>::new(None);
    let destination = Resource::new(move || location.pathname.get(), discovery_destination);
    Effect::new(move |_| match destination.try_get().flatten() {
        Some(Ok((resolved, destination_theme))) => {
            spawn_local(async move {
                match presentation.adopt(destination_theme).await {
                    Ok(crate::app::ThemeAdoption::Applied) => {
                        surface.set(Some(resolved));
                        error.set(None);
                    }
                    Ok(crate::app::ThemeAdoption::Superseded) => {}
                    Err(problem) => error.set(Some(problem.to_string())),
                }
            });
        }
        Some(Err(problem)) => {
            surface.set(None);
            error.set(Some(problem.to_string()));
        }
        None => {
            presentation.begin_navigation();
            error.set(None);
        }
    });
    view! {
        <Title text=move || {
            visible
                .get()
                .map_or_else(
                    || "Syndication feeds".to_owned(),
                    |surface| {
                        format!("Syndication feeds for {}", super::render::context_label(&surface))
                    },
                )
        } />
        {move || match visible.get() {
            Some(surface) => {
                super::render::body(
                        &surface,
                        &crate::app::render_theme_logo(&theme.get()),
                        &crate::app::render_theme_header(&theme.get()),
                    )
                    .inject_into(leptos::html::div().class("j-contents"))
                    .into_any()
            }
            None => {
                match error.get() {
                    Some(problem) => view! { <p class="error">{problem}</p> }.into_any(),
                    None => view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any(),
                }
            }
        }}
    }
}

async fn discovery_destination(
    path: String,
) -> WebResult<(FeedSurface, common::theme::PublishedThemePresentation)> {
    let surface = super::routes::discovery_surface(&path)
        .ok_or_else(|| crate::error::WebError::validation("Invalid Syndication Feed context"))?;
    let theme = discovery_theme(&surface).await?;
    Ok((surface, theme))
}

async fn discovery_theme(
    surface: &FeedSurface,
) -> WebResult<common::theme::PublishedThemePresentation> {
    use crate::timeline;
    use common::seed::TimelineOrder;

    match super::routes::listing_route_for_discovery(surface) {
        Some(route) => Ok(route.destination().await?.0),
        None => Ok(timeline::list_local_timeline(TimelinePageRequest {
            order: TimelineOrder::Newest,
            cursor: None,
            limit: None,
        })
        .await?
        .theme),
    }
}

/// Renders the `RSD` (`EditURI`) autodiscovery link for a user's `AtomPub`
/// publishing endpoint. Like [`FeedDiscovery`], it is invisible and only hoists
/// a `<link>` into the document head; editors such as `MarsEdit` follow it.
#[component]
pub fn RsdDiscovery<'a>(username: &'a Username) -> impl IntoView + use<> {
    view! {
        <Link
            rel="EditURI"
            type_="application/rsd+xml"
            title="AtomPub (RSD)"
            href=labels::rsd_href(username)
        />
    }
}

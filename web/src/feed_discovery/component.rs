use common::{
    feed::{FeedFormat, FeedSurface, canonicalize},
    username::Username,
};
use leptos::prelude::*;
use leptos_meta::Link;

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
            href=canonicalize(surface, FeedFormat::Rss)
        />
        <Link
            rel="alternate"
            type_="application/atom+xml"
            title=format!("{label} (Atom)")
            href=canonicalize(surface, FeedFormat::Atom)
        />
        <Link
            rel="alternate"
            type_="application/feed+json"
            title=format!("{label} (JSON Feed)")
            href=canonicalize(surface, FeedFormat::Json)
        />
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

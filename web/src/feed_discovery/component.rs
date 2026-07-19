use common::feed::{canonicalize, FeedFormat, FeedSurface};
use common::username::Username;
use leptos::prelude::*;
use leptos_meta::Link;

use super::labels::{rsd_href, surface_label};

/// Renders feed auto-discovery link tags for RSS, Atom, and JSON Feed.
/// The component itself is invisible; it hoists `<link>` tags into the document head.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
pub fn FeedDiscovery(surface: FeedSurface) -> impl IntoView {
    let label = surface_label(&surface);

    view! {
        <Link
            rel="alternate"
            type_="application/rss+xml"
            title=format!("{label} (RSS)")
            href=canonicalize(&surface, FeedFormat::Rss)
        />
        <Link
            rel="alternate"
            type_="application/atom+xml"
            title=format!("{label} (Atom)")
            href=canonicalize(&surface, FeedFormat::Atom)
        />
        <Link
            rel="alternate"
            type_="application/feed+json"
            title=format!("{label} (JSON Feed)")
            href=canonicalize(&surface, FeedFormat::Json)
        />
    }
}

/// Renders the `RSD` (`EditURI`) autodiscovery link for a user's `AtomPub`
/// publishing endpoint. Like [`FeedDiscovery`], it is invisible and only hoists
/// a `<link>` into the document head; editors such as `MarsEdit` follow it.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
pub fn RsdDiscovery(username: Username) -> impl IntoView {
    view! {
        <Link
            rel="EditURI"
            type_="application/rsd+xml"
            title="AtomPub (RSD)"
            href=rsd_href(&username)
        />
    }
}

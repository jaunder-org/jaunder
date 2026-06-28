// Leptos `view!` trees compile to deeply-nested tuple types; the editor's
// composer view (now carrying the audience picker) exceeds the default type
// recursion limit, so raise it for this crate.
#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
pub use common::username;

/// Wraps a `#[server]` function body in [`error::server_boundary`]: runs the
/// async block, and on an `InternalError` logs the structured failure (kind,
/// class, cause chain) and maps it to the public `WebError` so operator detail
/// never reaches the client. `$name` is the server-fn label used in that log and
/// in the error metric. Every server function routes its body through this.
#[macro_export]
macro_rules! boundary {
    ($name:expr, $body:block) => {
        $crate::error::server_boundary($name, async move $body).await
    };
}

pub mod audiences;
pub mod auth;
pub mod backup;
pub mod email;
pub mod error;
pub mod feed_discovery;
pub mod feed_events;
pub mod invites;
pub mod media;
pub mod pages;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod sessions;
pub mod site;
pub mod subscriptions;
pub mod tags;
pub mod viewer;

pub use error::server_resource;
pub use pages::App;

use leptos::prelude::*;
use leptos_meta::MetaTags;

#[must_use]
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <link rel="stylesheet" href="/style/jaunder.css" />
                <link rel="stylesheet" href="/style/jaunder-themes.css" />
                <AutoReload options=options.clone() />
                <HydrationScripts options />
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

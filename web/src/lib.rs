pub mod auth;
pub mod email;
pub mod invites;
pub mod pages;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod sessions;

pub use pages::App;

use leptos::prelude::*;
use leptos_meta::MetaTags;

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

//! Pure public Syndication Feed index markup, shared by the projector and CSR.
//! The same typed surface selects both the visible context and canonical feed URLs.

use common::feed::{self, FeedFormat, FeedSurface};
use maud::html;

use crate::html::Markup;

/// Names the exact public timeline represented by a discovery page.
#[must_use]
pub fn context_label(surface: &FeedSurface) -> String {
    match surface {
        FeedSurface::Site => "Local".to_owned(),
        FeedSurface::SiteTag { tag } => format!("site tag #{tag}"),
        FeedSurface::User { username } => format!("User ~{username}"),
        FeedSurface::UserTag { username, tag } => format!("User ~{username} tag #{tag}"),
    }
}

/// Renders one human-facing index for an existing public Syndication Feed context.
#[must_use]
pub fn body(surface: &FeedSurface, logo: &Markup, header: &Markup) -> Markup {
    let heading = format!("Syndication feeds for {}", context_label(surface));
    Markup::new(html! {
        (crate::app::render_theme_hero(
            &crate::topbar::render("Jaunder", &heading, None, &Markup::empty(), logo),
            header,
        ))
        div class="j-scroll" {
            div class="j-page" {
                ul {
                    @for (format, label) in [
                        (FeedFormat::Rss, "RSS"),
                        (FeedFormat::Atom, "Atom"),
                        (FeedFormat::Json, "JSON Feed"),
                    ] {
                        li { a href=(feed::canonicalize(surface, format)) { (label) } }
                    }
                }
            }
        }
    })
}

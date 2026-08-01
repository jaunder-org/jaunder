//! The home vertical's pure, projector-coincident render twin (ADR-0070's extra
//! leaf beside `component`): non-reactive markup only, so it stays host-tested
//! and coverage-measured while the reactive `HomePage` injects the very same
//! bytes.

use maud::html;

use crate::html::Markup;

/// The home page hero block (constant copy). Composed into
/// [`render_masthead`] — the one source the projector and the reactive
/// `home::HomePage` both render (ADR-0041 §2), so there is no `view!` twin.
#[must_use]
fn render_hero() -> Markup {
    Markup::new(html! {
        div class="j-hero" {
            h1 { "One timeline. Every protocol." }
            p {
                "Jaunder is a self-hosted social client that reads from ActivityPub, "
                "AT Protocol, RSS, Atom, and JSON Feed \u{2014} and publishes back out to "
                "the ones you choose. Below: what\u{2019}s been posted from this instance."
            }
        }
    })
}

/// The home page masthead — the topbar (with the anonymous Sign-in / Register
/// links) then the hero. The single source both the projector
/// (`crate::posts::render::render_body`) and the reactive `home::HomePage` render,
/// so coincidence holds by construction (ADR-0041 §2) — no `view!` twin to drift.
/// The links carry `j-anon-only` so the authed owner's pre-painted masthead hides
/// them (ADR-0044); an anonymous viewer (no `html.authed`) still sees them.
#[must_use]
pub(crate) fn render_masthead() -> Markup {
    let cta = Markup::new(html! {
        a href="/login" class="j-btn j-anon-only" { "Sign in" }
        a href="/register" class="j-btn is-primary j-anon-only" { "Register" }
    });
    Markup::new(html! {
        (crate::topbar::render(
            "jaunder.local",
            Some("Read-only \u{00b7} posts originating on this instance"),
            &cta,
        ))
        (render_hero())
    })
}

#[cfg(test)]
mod tests {
    use super::render_masthead;

    #[test]
    fn home_masthead_has_topbar_hero_and_anon_only_cta() {
        let markup = render_masthead();
        let html = markup.as_str();
        assert!(html.contains("<h1>jaunder.local</h1>"), "{html}");
        assert!(
            html.contains("<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>"
            ),
            "{html}"
        );
        assert!(html.contains("<div class=\"j-hero\">"), "{html}");
    }
}

//! The Local vertical's pure, projector-coincident render twin (ADR-0070's extra leaf
//! beside `component`): non-reactive markup only, so it stays host-tested and
//! coverage-measured while the reactive `LocalPage` injects the very same bytes.

use maud::html;

use crate::html::Markup;

/// The Local page masthead — the topbar with the anonymous Sign-in / Register links.
/// The single source both the projector (`crate::posts::render::body`) and reactive
/// `local::LocalPage` render, so coincidence holds by construction (ADR-0041 §2) — no
/// `view!` twin to drift. The links carry `j-anon-only`; an anonymous viewer still
/// sees them.
#[must_use]
pub(crate) fn masthead(logo: &Markup) -> Markup {
    let cta = Markup::new(html! {
        a href="/login" class="j-btn j-anon-only" { "Sign in" }
        a href="/register" class="j-btn is-primary j-anon-only" { "Register" }
    });
    Markup::new(html! {
        (crate::topbar::render(
            "jaunder.local",
            Some("Read-only \u{00b7} posts originating on this instance"),
            &cta,
            logo,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::masthead;
    use crate::html::Markup;

    #[test]
    fn local_masthead_has_topbar_and_anon_only_cta_without_hero() {
        let markup = masthead(&Markup::empty());
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
        assert!(!html.contains("<div class=\"j-hero\">"), "{html}");
        assert!(!html.contains("One timeline. Every protocol."), "{html}");
    }
}

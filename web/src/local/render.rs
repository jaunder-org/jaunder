//! The Local vertical's pure, projector-coincident render twin (ADR-0070's extra leaf
//! beside `component`): non-reactive markup only, so it stays host-tested and
//! coverage-measured while the reactive `LocalPage` injects the very same bytes.

use common::site::SiteIdentity;
use maud::html;

use crate::html::Markup;

/// The Local page masthead — the topbar with the anonymous Sign-in / Register links.
/// The single source both the projector (`crate::posts::render::body`) and reactive
/// `local::LocalPage` render, so coincidence holds by construction (ADR-0041 §2) — no
/// `view!` twin to drift. The links carry `j-anon-only`; an anonymous viewer still
/// sees them.
#[must_use]
pub(crate) fn masthead(identity: &SiteIdentity, logo: &Markup) -> Markup {
    let cta = Markup::new(html! {
        a href="/login" class="j-btn j-anon-only" { "Sign in" }
        a href="/register" class="j-btn is-primary j-anon-only" { "Register" }
    });
    Markup::new(html! {
        (crate::topbar::render(
            identity.title.as_ref(),
            identity.title.as_ref(),
            identity.tagline.as_ref().map(AsRef::as_ref),
            &cta,
            logo,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::masthead;
    use crate::html::Markup;
    use common::site::SiteIdentity;

    fn identity(tagline: Option<&str>) -> SiteIdentity {
        SiteIdentity {
            title: "Jaunder Sandbox".parse().unwrap(),
            tagline: tagline.map(str::parse).transpose().unwrap(),
            base_url: None,
        }
    }

    #[test]
    fn local_masthead_uses_identity_and_has_anon_only_cta_without_hero() {
        let markup = masthead(
            &identity(Some("Thoughtful <publishing>.")),
            &Markup::empty(),
        );
        let html = markup.as_str();
        assert!(
            html.contains("data-jaunder-part=\"site-title\">Jaunder Sandbox"),
            "{html}"
        );
        assert!(html.contains("<h1>Jaunder Sandbox</h1>"), "{html}");
        assert!(html.contains("Thoughtful &lt;publishing&gt;."), "{html}");
        assert_eq!(
            html.matches("data-jaunder-part=\"masthead\"").count(),
            1,
            "{html}"
        );
        assert_eq!(
            html.matches("data-jaunder-part=\"site-title\"").count(),
            1,
            "{html}"
        );
        assert!(
            html.find("data-jaunder-part=\"site-title\"").unwrap()
                < html.find("<h1>Jaunder Sandbox</h1>").unwrap(),
            "site identity must precede the Local heading in accessible source order: {html}"
        );
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

    #[test]
    fn local_masthead_renders_valid_tagline_matrix_representatives_as_text() {
        // Parser tests own rejection/normalization. This host-rendering boundary keeps
        // one valid scalar limit, interior-whitespace, and Unicode representative
        // auditable without duplicating the parser's matrix.
        for tagline in [
            "x".repeat(280),
            "Interior  whitespace".to_owned(),
            "Привет 🌍".to_owned(),
        ] {
            let html = masthead(&identity(Some(&tagline)), &Markup::empty()).into_string();
            assert!(
                html.contains(&format!(r#"<div class="j-sub">{tagline}</div>"#)),
                "valid tagline remains text in Local masthead: {html}"
            );
        }
    }

    #[test]
    fn local_masthead_omits_absent_tagline() {
        let html = masthead(&identity(None), &Markup::empty()).into_string();
        assert!(!html.contains("j-sub"), "{html}");
    }
}

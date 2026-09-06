use maud::html;

use crate::html::Markup;

/// The public masthead, mirroring the reactive [`Topbar`].
///
/// `right` fills the `j-topbar-right` slot (e.g. the home Sign-in / Register
/// buttons). It is a [`Markup`], so the "this is trusted HTML" claim is carried by
/// the type rather than by a comment asking callers to be careful; `title`/`sub`
/// are plain text and maud escapes them.
#[must_use]
pub(crate) fn render(title: &str, sub: Option<&str>, right: &Markup) -> Markup {
    Markup::new(html! {
        header class="j-topbar" data-jaunder-part="masthead" {
            div {
                span class="j-site-identity" data-jaunder-part="site-title" { "Jaunder" }
                h1 { (title) }
                @if let Some(s) = sub {
                    div class="j-sub" { (s) }
                }
            }
            div class="j-topbar-right" { (right) }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::html::Markup;

    // Renderer regression pins, NOT claims about the reactive `Topbar`: under CSR
    // the component builds DOM nodes and emits no bytes to compare against, so no
    // host test can verify correspondence. Coincidence is proven by
    // `expectNoShiftAcrossMount` (end2end/tests/layout-shift.ts).

    #[test]
    fn topbar_with_sub_markup_is_stable() {
        assert_eq!(
            render("Title", Some("Subtitle"), &Markup::empty()),
            "<header class=\"j-topbar\" data-jaunder-part=\"masthead\"><div>\
             <span class=\"j-site-identity\" data-jaunder-part=\"site-title\">Jaunder</span>\
             <h1>Title</h1><div class=\"j-sub\">Subtitle</div></div>\
             <div class=\"j-topbar-right\"></div></header>"
        );
    }

    #[test]
    fn topbar_without_sub_markup_is_stable() {
        assert_eq!(
            render("Title", None, &Markup::empty()),
            "<header class=\"j-topbar\" data-jaunder-part=\"masthead\"><div>\
             <span class=\"j-site-identity\" data-jaunder-part=\"site-title\">Jaunder</span>\
             <h1>Title</h1></div><div class=\"j-topbar-right\"></div></header>"
        );
    }
}

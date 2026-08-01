//! HTML text escaping — the crate's one low-level markup primitive.
//!
//! Deliberately cross-cutting rather than owned by a widget or vertical: every pure
//! markup builder (`app::render`, `avatar::markup`, `posts::render`,
//! `taglist::markup`, `topbar::markup`) interpolates untrusted text and must escape
//! it identically. Plain-string building only — no leptos reactivity — so
//! `reactive_graph` never sits on the public request path (the #173 escape).

use maud::Render;

/// A rendered HTML fragment — the render layer's currency and its **only**
/// trusted-HTML carrier.
///
/// Render fns return `Markup` rather than `String`, and `Markup` is the only thing
/// that composes into a `maud::html!` unescaped. So a hand-built string cannot reach
/// the output without passing the crate's single raw door,
/// `Markup::from_rendered_html` — the compiler enforcing what would otherwise need a
/// scanner. (That door arrives with its first caller, in the posts conversion; adding
/// it costs an entry in the `raw-html-door` gate's allowlist.)
///
/// Deliberately shadows `maud::Markup` (its `PreEscaped<String>` alias) inside this
/// crate: `Markup` is the word a `web` reader should reach for, and `maud::Markup` is
/// never imported. Do not glob-import maud, or the two collide.
///
/// Wraps the rendered `String` rather than `maud::Markup` because `PreEscaped`
/// implements neither `PartialEq` nor `Eq`, which the pinned render goldens need. The
/// invariant is unchanged: the field is rendered markup, and only the three
/// constructors below can mint one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Markup(String);

impl Markup {
    /// Wrap a rendered `html!` fragment: `Markup::new(html! { … })`.
    #[must_use]
    pub(crate) fn new(markup: maud::Markup) -> Self {
        Self(markup.into_string())
    }

    /// The rendered markup as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume the fragment, yielding the rendered markup.
    ///
    /// This is the exit to untyped-string land — the projector's response body and
    /// leptos `inner_html=`. Both sinks receive a value that was `Markup`, which is
    /// what carries the trust across those boundaries.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl Render for Markup {
    /// A `Markup` is by definition already rendered, so it goes in verbatim.
    fn render_to(&self, buffer: &mut String) {
        buffer.push_str(self.as_str());
    }
}

/// Escape text for safe interpolation into HTML element or attribute content.
pub(crate) fn escape_html<S: AsRef<str>>(input: S) -> String {
    let input = input.as_ref();
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape_html;

    #[test]
    fn escape_replaces_markup_metacharacters() {
        assert_eq!(escape_html("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
    }
}

#[cfg(test)]
mod markup_tests {
    use super::Markup;
    use maud::html;

    /// Text-slot security property: a hostile payload contributes no `<` of its own
    /// and cannot open an element. Stated in forbidden characters rather than
    /// expected bytes, so a safe change in escaping *style* does not fail this test
    /// — a test that cries wolf teaches its readers to re-bless it.
    #[test]
    fn hostile_text_payload_contributes_no_markup() {
        let hostile = r#"' " & < > </script>"#;
        let benign = "aaaaaaaaaaaaaaaaaaa"; // same length, no metacharacters
        let render = |s: &str| Markup::new(html! { p { (s) } }).into_string();

        let hostile_out = render(hostile);
        let benign_out = render(benign);

        assert_eq!(
            hostile_out.matches('<').count(),
            benign_out.matches('<').count(),
            "payload contributed an angle bracket: {hostile_out}"
        );
        assert!(!hostile_out.contains("<script"), "{hostile_out}");
        assert!(!hostile_out.contains("</script"), "{hostile_out}");
    }

    /// Attribute-slot security property: the payload cannot terminate the attribute
    /// it sits in, so it cannot introduce a sibling attribute.
    #[test]
    fn hostile_attribute_payload_cannot_terminate_the_attribute() {
        let hostile = r#"x" onerror="alert(1)"#;
        let out = Markup::new(html! { img alt=(hostile); }).into_string();
        let (value, after_value) = out
            .split_once("alt=\"")
            .expect("alt attribute present")
            .1
            .split_once('"')
            .expect("attribute terminator present");

        // The payload stays *inside* the quoted value: no raw quote to close it
        // early, and nothing it contributed lands after the terminator where it
        // would parse as a further attribute. Asserting on position, not presence —
        // `onerror` appearing escaped *within* the value is the safe outcome, so a
        // bare `!out.contains("onerror")` would fail on correct behavior.
        assert!(!value.contains('"'), "raw quote survived in {out}");
        assert!(
            !after_value.contains("onerror"),
            "attribute broke out: {out}"
        );
    }

    /// A `Markup` composes into a surrounding fragment verbatim — it is already
    /// rendered, so it must not be escaped a second time.
    #[test]
    fn markup_composes_verbatim_inside_html_macro() {
        let inner = Markup::new(html! { em { "x & y" } });
        assert_eq!(
            Markup::new(html! { div { (inner) } }).into_string(),
            "<div><em>x &amp; y</em></div>"
        );
    }

    #[test]
    fn markup_is_cloneable_and_comparable() {
        let m = Markup::new(html! { b { "x" } });
        assert_eq!(m.clone(), m);
    }
}

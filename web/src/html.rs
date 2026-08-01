//! [`Markup`] — the render layer's currency and its one trusted-HTML door.
//!
//! Deliberately cross-cutting rather than owned by a widget or vertical: every pure
//! markup builder (`app::render`, `avatar::markup`, `posts::render`,
//! `sidebar::markup`, `taglist::markup`, `topbar::markup`, …) returns `Markup`, and
//! `Markup` is the only thing that composes into a `maud::html!` unescaped. Escaping
//! is therefore no longer a rule anyone has to remember: text interpolated into
//! `html!` is escaped by the macro, and the one way to emit raw HTML is
//! [`Markup::from_rendered_html`], which the `raw-html-door` gate pins to this file.
//! (This replaced a hand-rolled escaper that every builder had to remember to call
//! at every interpolation — #333.)
//!
//! **Non-reactive markup only — no leptos reactivity**, so `reactive_graph` never
//! sits on the public request path (the #173 escape, ADR-0040). maud preserves that
//! property: `html!` is a compile-time macro that builds a string, with no runtime.

use common::render::RenderedHtml;
use maud::{PreEscaped, Render};

/// A rendered HTML fragment — the render layer's currency and its **only**
/// trusted-HTML carrier.
///
/// Render fns return `Markup` rather than `String`, and `Markup` is the only thing
/// that composes into a `maud::html!` unescaped. So a hand-built string cannot reach
/// the output without passing the crate's single raw door,
/// [`Markup::from_rendered_html`] — the compiler enforcing what would otherwise need
/// a scanner.
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

    /// The empty fragment, for an absent optional slot.
    #[must_use]
    pub(crate) fn empty() -> Self {
        Self(String::new())
    }

    /// Carry an already-sanitized [`RenderedHtml`] into the markup layer unescaped.
    ///
    /// **This is the crate's one raw door.** The `raw-html-door` static check fails
    /// the build on any other `PreEscaped` under `web/src`, including inside macro
    /// bodies, so a second door has to argue for itself in review.
    #[must_use]
    pub(crate) fn from_rendered_html(html: &RenderedHtml) -> Self {
        // XSS SAFETY: `RenderedHtml`'s invariant is established by sanitization
        // (ADR-0079) — this only inherits it, so no escaping is owed here.
        //
        // Because `Markup` already stores the rendered string, wrapping in
        // `PreEscaped` and unwrapping is a no-op on the bytes. It is kept because it
        // is what "this string is markup, not text" is *spelled* as in maud, and it
        // is the token the `raw-html-door` gate keys on — which is what catches the
        // real hazard, a `PreEscaped(user_input)` appearing anywhere else in `web`.
        Self(PreEscaped(html.as_ref()).into_string())
    }

    /// The rendered markup as a string slice.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
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
    ///
    /// This impl is the whole reason the type exists: without a `Render` that emits
    /// raw, `html! { (render_avatar(…)) }` would *escape* the nested markup. The
    /// escaping guarantee itself lives here and in the single `PreEscaped` door —
    /// **not** in how convertible `Markup` is to a string, which is why the
    /// conversions below cost nothing in safety.
    fn render_to(&self, buffer: &mut String) {
        buffer.push_str(self.as_str());
    }
}

impl AsRef<str> for Markup {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<Markup> for String {
    fn from(markup: Markup) -> Self {
        markup.0
    }
}

/// Compare rendered markup against a literal, so the pinned render goldens read
/// `assert_eq!(render(…), "<div>…</div>")` without an `.as_str()` at every site.
impl PartialEq<&str> for Markup {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Markup {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

// The reversed impls (`&str == Markup`, `String == Markup`) are deliberately absent:
// every golden reads `assert_eq!(render(…), "literal")`, so nothing calls them and
// they would be uncovered lines carrying no weight.

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

    /// The string conversions are ergonomic only — they must not change what
    /// `html!` does with a `Markup`. Escaping is decided by the `Render` impl, so a
    /// nested `Markup` still goes in verbatim no matter how convertible it is.
    #[test]
    fn string_conversions_do_not_affect_how_html_renders_markup() {
        let inner = Markup::new(html! { em { "a & b" } });
        assert_eq!(inner.as_ref(), "<em>a &amp; b</em>");
        assert_eq!(String::from(inner.clone()), "<em>a &amp; b</em>");
        // Still raw when nested, not re-escaped.
        assert_eq!(
            Markup::new(html! { div { (inner) } }),
            "<div><em>a &amp; b</em></div>"
        );
    }

    #[test]
    fn markup_compares_against_str_and_string_literals() {
        let m = Markup::new(html! { b { "x" } });
        assert_eq!(m, "<b>x</b>");
        assert_eq!(m, "<b>x</b>".to_string());
        assert_ne!(m, "<b>y</b>");
    }

    #[test]
    fn markup_is_cloneable_and_comparable() {
        let m = Markup::new(html! { b { "x" } });
        assert_eq!(m.clone(), m);
    }
}

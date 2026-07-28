//! HTML text escaping — the crate's one low-level markup primitive.
//!
//! Deliberately cross-cutting rather than owned by a widget or vertical: every pure
//! markup builder (`app::render`, `avatar::markup`, `posts::render`,
//! `taglist::markup`, `topbar::markup`) interpolates untrusted text and must escape
//! it identically. Plain-string building only — no leptos reactivity — so
//! `reactive_graph` never sits on the public request path (the #173 escape).

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

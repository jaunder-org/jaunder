//! Avatar — the initials chip. The pure [`render`] (server projector) and the
//! reactive [`Avatar`] (CSR client) are twins: the same `<div class="j-av">`
//! markup produced two ways. Co-located per ADR-0056.

use leptos::prelude::*;

use crate::render::escape_html;

/// Derives `(initials, hue)` from a display name. `initials`: first character of
/// each of the first two whitespace-separated words, uppercased. `hue`: sum of all
/// char codes mod 360. Shared by the reactive [`Avatar`] and the pure [`render`]
/// twin so a seeded avatar and its reactive re-render coincide.
#[must_use]
fn avatar_parts(name: &str) -> (String, u32) {
    let initials: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|word| word.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let hue: u32 = name.chars().fold(0u32, |acc, c| acc + c as u32) % 360;
    (initials, hue)
}

/// One avatar chip as `<div class="j-av" …>`, byte-identical to the reactive
/// [`Avatar`] component's output for the same `(name, size)`.
#[must_use]
pub(crate) fn render(name: &str, size: u32) -> String {
    let (initials, hue) = avatar_parts(name);
    // Integer equivalent of `(size as f32 * 0.36).round()`, avoiding float casts;
    // `+ 50` gives round-half-up. `size` is a small avatar dimension.
    let font_size = (size * 36 + 50) / 100;
    format!(
        "<div class=\"j-av\" style=\"width:{size}px;height:{size}px;background:oklch(0.58 0.07 {hue});font-size:{font_size}px\">{initials}</div>",
        initials = escape_html(&initials),
    )
}

/// The reactive half of the twin: an initials chip derived from `name`.
/// Twins [`render`] — keep their markup coincident.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
#[component]
pub fn Avatar(name: String, #[prop(default = 38)] size: u32) -> impl IntoView {
    let (initials, hue) = avatar_parts(&name);
    // Integer equivalent of `(size as f32 * 0.36).round()`; must match the pure
    // `render` twin so SSR and reactive output coincide.
    let font_size = (size * 36 + 50) / 100;
    let style = format!(
        "width:{size}px;height:{size}px;background:oklch(0.58 0.07 {hue});font-size:{font_size}px"
    );
    view! {
        <div class="j-av" style=style>
            {initials}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{avatar_parts, render};

    #[test]
    fn avatar_matches_reactive_component_markup() {
        // Must stay byte-identical to the reactive `Avatar` for size 38.
        let (initials, hue) = avatar_parts("Mara Ek");
        assert_eq!(initials, "ME");
        let html = render("Mara Ek", 38);
        assert_eq!(
            html,
            format!(
                "<div class=\"j-av\" style=\"width:38px;height:38px;background:oklch(0.58 0.07 {hue});font-size:14px\">ME</div>"
            )
        );
    }

    #[test]
    fn avatar_parts_single_word() {
        let (initials, _hue) = avatar_parts("Mara");
        assert_eq!(initials, "M");
    }

    #[test]
    fn avatar_parts_two_words() {
        let (initials, _hue) = avatar_parts("Mara Ek");
        assert_eq!(initials, "ME");
    }

    #[test]
    fn avatar_parts_more_than_two_words_uses_first_two() {
        let (initials, _hue) = avatar_parts("Mara Jane Ek");
        assert_eq!(initials, "MJ");
    }

    #[test]
    fn avatar_parts_empty_name() {
        let (initials, hue) = avatar_parts("");
        assert_eq!(initials, "");
        assert_eq!(hue, 0);
    }

    #[test]
    fn avatar_parts_hue_is_in_range() {
        let (_initials, hue) = avatar_parts("Some User");
        assert!(hue < 360);
    }

    #[test]
    fn avatar_parts_hue_is_deterministic() {
        let (_, h1) = avatar_parts("Mara Ek");
        let (_, h2) = avatar_parts("Mara Ek");
        assert_eq!(h1, h2);
    }

    #[test]
    fn avatar_parts_hue_differs_for_different_names() {
        let (_, h1) = avatar_parts("Alice");
        let (_, h2) = avatar_parts("Bob");
        assert_ne!(h1, h2);
    }
}

//! Avatar — the initials chip. The pure [`render`] (server projector) and the
//! reactive [`Avatar`] (CSR client) are twins: the same `<div class="j-av">`
//! markup produced two ways. Co-located per ADR-0056.

use leptos::prelude::*;

use crate::render::{avatar_parts, escape_html};

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
    use super::render;
    use crate::render::avatar_parts;

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
}

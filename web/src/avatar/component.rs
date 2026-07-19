use leptos::prelude::*;

use super::markup::avatar_parts;

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

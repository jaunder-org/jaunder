use common::username::Username;
use leptos::prelude::*;

use super::markup;

/// The reactive half of the twin: an initials chip derived from `name`.
/// Twins [`render`] — keep their markup coincident.
#[component]
pub fn Avatar<'a>(name: &'a Username, #[prop(default = 38)] size: u32) -> impl IntoView + use<> {
    let (initials, hue) = markup::avatar_parts(name);
    // Integer equivalent of `(size as f32 * 0.36).round()`; must match the pure
    // `render` twin so the projector paint and this reactive component coincide.
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

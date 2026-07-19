//! Icon — one inline SVG glyph. The pure [`render`] (server projector) and the
//! reactive [`Icon`] (CSR client) are twins: the same `<svg class="j-icon">`
//! markup produced two ways. Co-located per ADR-0056.

#[cfg(target_arch = "wasm32")]
mod component;
mod markup;

/// SVG path `d` strings — re-exported from the pure `render` layer so the reactive
/// [`Icon`] component and the pure [`render`] twin share one source of truth.
pub use crate::render::Icons;
#[cfg(target_arch = "wasm32")]
pub use component::Icon;
pub(crate) use markup::render;

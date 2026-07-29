//! Icon — one inline SVG glyph. The pure [`render`] (server projector) and the
//! reactive [`Icon`] (CSR client) are twins: the same `<svg class="j-icon">`
//! markup produced two ways. Co-located per ADR-0056.

#[cfg(target_arch = "wasm32")]
mod component;
mod markup;
mod paths;

#[cfg(target_arch = "wasm32")]
pub use component::Icon;
pub(crate) use markup::render;
/// SVG path `d` strings — the one source of truth the reactive [`Icon`] component
/// and the pure [`render`] twin share.
pub use paths::Icons;

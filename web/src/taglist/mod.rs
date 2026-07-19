//! `TagList` — a post's footer tag chips. The pure [`render`] (server projector,
//! injected via `inner_html`) and the reactive [`TagList`] (CSR client, the
//! authored post view the projector never renders) are twins: the same
//! `<span class="j-tag-list">` markup produced two ways. Co-located per ADR-0056.

#[cfg(target_arch = "wasm32")]
mod component;
mod markup;

#[cfg(target_arch = "wasm32")]
pub use component::TagList;
pub(crate) use markup::render;

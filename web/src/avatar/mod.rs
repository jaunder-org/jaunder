//! Avatar — the initials chip. The pure [`render`] (server projector) and the
//! reactive [`Avatar`] (CSR client) are twins: the same `<div class="j-av">`
//! markup produced two ways. Co-located per ADR-0056.

#[cfg(target_arch = "wasm32")]
mod component;
mod markup;

#[cfg(target_arch = "wasm32")]
pub use component::Avatar;
pub(crate) use markup::render;

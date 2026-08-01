//! The app shell vertical (#330, ADR-0070): `App` + the Router/route table and
//! their pure projector twin. `render` is the host-compiled shell projector
//! (shared with `server::projector`); `component` is the wasm-only reactive shell.

mod render;
pub use render::{
    render_head, render_shell, DEFAULT_THEME, DISCOVERY_MARKER_ATTR, PREPAINT_SCRIPT, SPA_SHELL,
};

/// The render layer's markup type, re-exported so cross-crate consumers of
/// [`render_head`]/[`render_shell`] (the projector) can name what they receive.
/// Trust is type-carried across that boundary: the projector is handed a `Markup`,
/// not a bare `String`.
pub use crate::html::Markup;

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::App;

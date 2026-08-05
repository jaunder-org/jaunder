//! The app shell vertical (#330, ADR-0070): `App` + the Router/route table and
//! their pure projector twin. `render` is the host-compiled shell projector
//! (shared with `server::projector`); `component` is the wasm-only reactive shell.

mod render;
pub use render::{
    DEFAULT_THEME, DISCOVERY_MARKER_ATTR, PREPAINT_SCRIPT, SPA_SHELL, render_head, render_shell,
};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::App;

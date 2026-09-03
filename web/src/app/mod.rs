//! The app shell vertical (#330, ADR-0070): `App` + the Router/route table and
//! their pure projector twin. `render` is the host-compiled shell projector
//! (shared with `server::projector`); `component` is the wasm-only reactive shell.

mod seed;
pub use seed::decode_projector_seed;

mod render;
pub use render::{
    DISCOVERY_MARKER_ATTR, MODULE_BEFORE_INIT_MARK, PREPAINT_SCRIPT, render_head, render_shell,
};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::{App, public_theme};

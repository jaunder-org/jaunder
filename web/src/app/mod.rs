//! The app shell vertical (#330, ADR-0070): `App` + the Router/route table and
//! their pure projector twin. `render` is the host-compiled shell projector
//! (shared with `server::projector`); `component` is the wasm-only reactive shell.

mod seed;
pub use seed::decode_projector_seed;

#[cfg(any(target_arch = "wasm32", test))]
mod theme;

#[cfg(target_arch = "wasm32")]
mod theme_storage;

mod render;
pub use render::{
    DEFAULT_THEME, DISCOVERY_MARKER_ATTR, EARLY_WASM_FETCH_SCRIPT, GLUE_URL,
    MODULE_BEFORE_INIT_MARK, PREPAINT_SCRIPT, SPA_SHELL, WASM_URL, render_head, render_shell,
};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::App;

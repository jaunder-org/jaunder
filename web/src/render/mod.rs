//! Shared pure, non-reactive HTML leaf primitives for the web crate.
//!
//! Now empty: every primitive that lived here — HTML escaping, SVG icon path data,
//! tag-link context, the home hero/masthead, the projector "Load more" placeholder,
//! and the byte-size formatter — has moved to its co-located home (#658). The
//! page-frame shell projector (`render_shell`/`render_head` + the shell constants)
//! moved to `crate::app::render` with the reactive shell it twins (#330). The module
//! itself is deleted in the next commit.

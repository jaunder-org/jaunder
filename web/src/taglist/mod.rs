//! A post's footer tag chips: the pure [`render`], injected via `inner_html` by both
//! the server projector and the CSR client. Co-located per ADR-0056.
//!
//! There is deliberately no reactive component twin: both sides paint these chips
//! through `render` via the ADR-0041 seam (`posts::render`'s `PostView` carries
//! `tags` and `tag_ctx`, #181), so there is one renderer and nothing to keep
//! coincident (#301).

mod context;
mod markup;

pub use context::TagCtx;
pub(crate) use markup::render;

//! A post's footer tag chips: the pure [`render`], injected via `inner_html` by both
//! the server projector and the CSR client. Co-located per ADR-0056.
//!
//! This leaf once held a reactive component twin alongside `render`, so the same
//! `<span class="j-tag-list">` markup was produced two ways. #181 moved the CSR
//! authored-post view onto the shared pure path (`posts::render`'s `PostView` carries
//! `tags` and `tag_ctx`), which is the ADR-0041 seam — from then on both sides painted
//! these chips through `render`, and the component had no callers. It was carried
//! along by two later relocations before being deleted in #301. There is one renderer
//! now, so there is nothing left to keep coincident.

mod context;
mod markup;

pub use context::TagCtx;
pub(crate) use markup::render;

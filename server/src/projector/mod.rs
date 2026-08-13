//! Non-reactive server-side HTML for the public discoverability routes (#178).
//!
//! When a public URL resolves to **public** content, the projector emits one
//! cacheable document (semantic content, an embedded `#jaunder-seed` data blob,
//! and the CSR boot script) with **no `reactive_graph` on the request path**,
//! the #173 escape and the same posture as the feed handlers. Every page renders
//! the **anonymous** view (`ViewerIdentity::Anonymous`), so the bytes are
//! identical per URL for every visitor — CDN-cacheable.
//!
//! When the URL has no anonymous-public content (a draft the author must see,
//! a not-yet-existing post, an unparseable segment), the projector serves the
//! **SPA shell** instead — identical to the pre-projector fallback — so the CSR
//! client boots and resolves it with the viewer's session (drafts, client-side
//! 404s, and the authed owner's affordances all keep working). The projector
//! only ever *adds* server rendering for content that is already public.
//!
//! `register`/`document`/the handlers are always compiled (so they stay
//! unit-testable and covered under default features); they are wired into the
//! axum router only under `--features csr` (see `create_router`), ahead of the
//! static-SPA fallback.

mod document;
mod handlers;
mod shell;

pub use document::document;
pub use handlers::register;
pub use shell::Shell;

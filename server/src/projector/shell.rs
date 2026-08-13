use std::sync::Arc;

/// The static SPA shell (`index.html`) the projector falls back to when a public
/// URL has no anonymous-public content. Cheap to clone (shared `Arc`).
#[derive(Clone)]
pub struct Shell(pub Arc<str>);

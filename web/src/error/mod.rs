#[cfg(feature = "server")]
mod server;
mod wire;

// The server-side error carrier lives in `host` (ADR-0058); `web` keeps only the
// wire type and the `kind → WebError` projection. Re-exported so every vertical's
// `InternalError::storage(…)`/`?` call site names it unchanged through `web::error`.
// `project` keeps its `crate::error::project` path for `web`'s own tests (the only
// in-crate caller outside this module — `auth::server`'s projection assertions);
// production code reaches it through `server_boundary`.
#[cfg(all(test, feature = "server"))]
pub(crate) use server::project;
#[cfg(feature = "server")]
pub use server::{
    ErrorClass, ErrorKind, InternalError, InternalResult, SwallowedSource, report_swallowed,
    server_boundary,
};
pub use wire::{WebError, WebResult};

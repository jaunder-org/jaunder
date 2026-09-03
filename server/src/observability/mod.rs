//! Server observability assembly: diagnostics, telemetry initialization, and HTTP middleware.

mod diagnostics;
mod http;
mod initialization;

pub use http::with_http_observability;
pub use initialization::init_server_tracing;

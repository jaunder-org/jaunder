//! Host-only server-fn registration surface for integration tests.
//!
//! `#[macros::server]` contributes one thunk per server fn to
//! [`SERVER_FN_REGISTRATIONS`]. Test harnesses call [`register_all`] once to make
//! every generated `ServerFn` route reachable when `web` is linked as an rlib.

use linkme::distributed_slice;

/// One generated server-fn registration action.
pub type Registration = fn();

/// Registration thunks emitted by `#[macros::server]`.
#[distributed_slice]
pub static SERVER_FN_REGISTRATIONS: [Registration];

/// Register every generated server fn with `server_fn`'s axum registry.
pub fn register_all() {
    for register in SERVER_FN_REGISTRATIONS {
        register();
    }
}

/// Number of registration thunks retained in this binary.
#[must_use]
pub fn registration_count() -> usize {
    SERVER_FN_REGISTRATIONS.len()
}

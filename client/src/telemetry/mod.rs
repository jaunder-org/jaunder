//! One-flight delivery for swallowed browser-error diagnostics.
//!
//! [`Reporter`] is the host-testable state machine: it emits to its local
//! [`ConsoleSink`] before consulting the flight slot, drops rather than queues a
//! concurrent event, and gives the transport the only callback that can clear
//! the slot. The browser adapter owns one reporter for the page and keeps all
//! fetch construction and self-failure handling inside this module.

mod reporter;

pub use reporter::{Completion, ConsoleSink, Reporter, Transport};

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::report_swallowed;

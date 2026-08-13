//! Browser performance marks emitted from Rust (#794).
//!
//! The CSR boot is a black box from outside the wasm: the e2e harness can see
//! when the navigation committed and when `data-mounted` appeared, but nothing
//! in between. These marks cut that interval into phases so `commit_to_mount`
//! can be decomposed rather than merely sized.
//!
//! Two properties the rest of the system leans on:
//!
//! - **Every name starts with [`MARK_PREFIX`].** The harness discovers marks by
//!   that prefix, never by a name list, so adding a mark here needs no change on
//!   the TypeScript side.
//! - **Emitting is unconditional** — no cargo feature guards [`mark`]. A
//!   feature-gated mark would mean the binary we measure is not the binary we
//!   ship.
//!
//! Unlike the rest of `client`, this module is compiled on the host too: the
//! names are plain `&str` data and the contract above is worth a host test. Only
//! the `web_sys` call is wasm-only, split at the `mod` wiring below per ADR-0070.
//! A missing `performance` API degrades to a no-op; boot never depends on it.

mod names;

pub use names::{BOOT_ENTRY, BOOT_MOUNT_DONE, BOOT_RENDER_START, BOOT_SEED_PARSED, MARK_PREFIX};

#[cfg(target_arch = "wasm32")]
mod emit;
#[cfg(target_arch = "wasm32")]
pub use emit::mark;

#[cfg(not(target_arch = "wasm32"))]
mod noop;
#[cfg(not(target_arch = "wasm32"))]
pub use noop::mark;

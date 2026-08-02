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

/// The discovery prefix. The e2e harness selects marks with
/// `name.startsWith(MARK_PREFIX)` and never enumerates names, so a new mark
/// costs one line here and nothing in TypeScript.
pub const MARK_PREFIX: &str = "jaunder.";
/// The wasm is running — its own first observable instant. Everything before it
/// (fetch, compile, instantiate) is derived from the gap since the commit.
pub const BOOT_ENTRY: &str = "jaunder.boot.entry";
/// The projector's seed blob has been read out of the DOM and deserialized.
pub const BOOT_SEED_PARSED: &str = "jaunder.boot.seed_parsed";
/// About to call `mount_to_body` — everything after this is leptos rendering.
pub const BOOT_RENDER_START: &str = "jaunder.boot.render_start";
/// `mount_to_body` has returned. Paired with `data-mounted`, which is set next.
pub const BOOT_MOUNT_DONE: &str = "jaunder.boot.mount_done";

#[cfg(target_arch = "wasm32")]
mod emit;
#[cfg(target_arch = "wasm32")]
pub use emit::mark;

#[cfg(not(target_arch = "wasm32"))]
mod noop;
#[cfg(not(target_arch = "wasm32"))]
pub use noop::mark;

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [&str; 4] = [
        BOOT_ENTRY,
        BOOT_SEED_PARSED,
        BOOT_RENDER_START,
        BOOT_MOUNT_DONE,
    ];

    #[test]
    fn every_boot_mark_carries_the_discovery_prefix() {
        for name in ALL {
            assert!(
                name.starts_with(MARK_PREFIX),
                "{name} is invisible to prefix discovery"
            );
        }
    }

    #[test]
    fn boot_mark_names_are_distinct() {
        let mut sorted = ALL.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ALL.len(), "duplicate mark name in {ALL:?}");
    }

    #[test]
    fn marking_off_the_browser_is_a_no_op() {
        for name in ALL {
            mark(name);
        }
    }
}

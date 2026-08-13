/// The discovery prefix. The e2e harness selects marks with
/// `name.startsWith(MARK_PREFIX)` and never enumerates names, so a new mark
/// costs one line here and nothing in TypeScript.
///
/// The prefix itself is the one thing that *is* spelled twice — the harness
/// declares its own copy in `end2end/tests/capture-trace.ts`, since no import
/// crosses into Node. The `xlang-literal` gate
/// (`xtask/src/steps/xlang_literal_check.rs`) fails `cargo xtask check` when the
/// two copies disagree (#767).
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

#[cfg(test)]
mod tests {
    use super::*;
    // `mark` lives behind `mod.rs`'s target-gated re-export, so it does not
    // arrive through the glob above.
    use crate::perf::mark;

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

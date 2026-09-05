//! The raw wasm size budget (#836).
//!
//! **Raw bytes, not brotli.** Compression governs transfer, but the wasm
//! compiler's input is the decompressed artifact, and compile is what dominates
//! the firefox/chromium boot gap (#818: 80.5–87.6% of it, at ~88 ms per MiB).
//! A budget on the compressed figure would be satisfied by a change that
//! compresses better while compiling slower. This will look wrong to anyone whose
//! instinct is "measure what users download" — it is deliberate.

use serde::Serialize;

/// Raw bytes of `pkg/jaunder.wasm` achieved after the SMTP relay operator UI
/// landed (#638), still using `wasm-opt -Oz`.
///
/// `validate` reports observed size as a drift against this. **A drift of a few
/// bytes is build noise, not erosion**: the artifact is not bit-reproducible
/// across builds — a docs-only commit was observed to move it by 13 bytes. Read
/// the drift for its order of magnitude, not its sign; kilobytes mean something
/// changed.
pub const WASM_RAW_ACHIEVED_BYTES: u64 = 2_700_875;

/// The ceiling `cargo xtask validate` enforces.
///
/// Headroom is **3.1%** over [`WASM_RAW_ACHIEVED_BYTES`]. The three optimisation
/// levels were re-measured on the SMTP-relay bundle:
///
/// | build                      | raw bytes |
/// | -------------------------- | --------- |
/// | `-Oz` (achieved)           | 2 700 875 |
/// | **ceiling**                | **2 785 000** |
/// | `-Os`                      | 2 824 602 |
/// | `-O2`                      | 2 864 716 |
///
/// The ceiling leaves ordinary headroom but remains below both weaker
/// optimisation levels, so losing `-Oz` still fails rather than being hidden by
/// the feature-driven recalibration.
///
/// Lower it deliberately, in the same commit as the win that earned it.
pub const WASM_RAW_CEILING_BYTES: u64 = 2_785_000;

#[derive(Debug, Serialize)]
pub struct BudgetVerdict {
    pub actual: u64,
    pub ceiling: u64,
    pub over: bool,
}

/// Compare a measured size against a ceiling. The ceiling is **inclusive** — a
/// build that lands exactly on it is not a regression.
pub fn check(actual: u64, ceiling: u64) -> BudgetVerdict {
    BudgetVerdict {
        actual,
        ceiling,
        over: actual > ceiling,
    }
}

/// What to print when the budget is blown: the numbers, and the one edit that
/// resolves it — so the reader does not have to guess whether the right response
/// is to shrink the bundle or to move the line.
pub fn failure_message(v: &BudgetVerdict) -> String {
    format!(
        "raw pkg/jaunder.wasm is {} bytes, over the {} byte ceiling by {}.\n\
         This budget is on RAW bytes, not brotli: raw is what the wasm compiler \
         reads, and compile time is what dominates the boot gap (#818).\n\
         If the growth is intended, raise WASM_RAW_CEILING_BYTES in \
         xtask/src/wasm_budget.rs deliberately, and say why in the commit. \
         If it is not, `cargo xtask audit-wasm --breakdown` attributes the bytes \
         to crates.",
        v.actual,
        v.ceiling,
        v.actual.saturating_sub(v.ceiling),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_ceiling_passes() {
        assert!(!check(1_000, 2_000).over);
    }

    #[test]
    fn exactly_at_ceiling_passes() {
        // Inclusive: a build that exactly hits the ceiling is not a regression.
        assert!(!check(2_000, 2_000).over);
    }

    #[test]
    fn over_ceiling_fails() {
        assert!(check(2_001, 2_000).over);
    }

    #[test]
    fn failure_message_states_actual_ceiling_and_the_remedy() {
        let m = failure_message(&check(3_000, 2_000));
        assert!(m.contains("3000"), "{m}");
        assert!(m.contains("2000"), "{m}");
        assert!(
            m.contains("WASM_RAW_CEILING_BYTES"),
            "must name the constant to change: {m}"
        );
        assert!(m.to_uppercase().contains("RAW"), "{m}");
    }

    /// Raw bytes of the shipped wasm at the weaker `wasm-opt` levels, remeasured
    /// on the SMTP-relay bundle. `NO_WASM_OPT_BYTES` retains the pre-#836
    /// historical guard. The next three tests run the real predicate over them.
    const NO_WASM_OPT_BYTES: u64 = 5_350_591;
    const O2_LEVEL_BYTES: u64 = 2_864_716;
    const OS_LEVEL_BYTES: u64 = 2_824_602;

    #[test]
    fn the_achieved_size_passes_its_own_budget() {
        // A ceiling equal to the achieved size is a strict ratchet, which #836
        // rejected — every innocent dependency bump would turn the gate red, and
        // the only available fix is to raise the number.
        let v = check(WASM_RAW_ACHIEVED_BYTES, WASM_RAW_CEILING_BYTES);
        assert!(!v.over, "{v:?}");
        assert!(
            v.ceiling > v.actual,
            "the budget must leave headroom, not sit exactly on the achieved size: {v:?}"
        );
    }

    #[test]
    fn a_downgrade_of_the_optimisation_level_fails_the_budget() {
        // The headroom is bounded on the far side too. Losing `-Oz` is the
        // likeliest way this win evaporates, so both weaker levels must land
        // above the ceiling rather than inside the tolerance.
        assert!(
            check(OS_LEVEL_BYTES, WASM_RAW_CEILING_BYTES).over,
            "a -Os build must not fit under the ceiling"
        );
        assert!(
            check(O2_LEVEL_BYTES, WASM_RAW_CEILING_BYTES).over,
            "a -O2 build must not fit under the ceiling"
        );
    }

    #[test]
    fn the_pre_836_bundle_would_fail_todays_budget() {
        assert!(
            check(NO_WASM_OPT_BYTES, WASM_RAW_CEILING_BYTES).over,
            "losing wasm-opt entirely must fail the gate"
        );
    }
}

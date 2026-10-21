//! The raw wasm size budget (#836).
//!
//! **Raw bytes, not brotli.** Compression governs transfer, but the wasm
//! compiler's input is the decompressed artifact, and compile is what dominates
//! the firefox/chromium boot gap (#818: 80.5–87.6% of it, at ~88 ms per MiB).
//! A budget on the compressed figure would be satisfied by a change that
//! compresses better while compiling slower. This will look wrong to anyone whose
//! instinct is "measure what users download" — it is deliberate.

use serde::Serialize;

/// Raw bytes of the manifest-selected WASM identity artifact after Manage Posts
/// added paginated filtering, cross-page selection, confirmation, and bulk
/// audience controls (#1625), still using `wasm-opt -Oz`.
///
/// `validate` reports observed size as a drift against this. **A drift of a few
/// bytes is build noise, not erosion**: the artifact is not bit-reproducible
/// across builds — a docs-only commit was observed to move it by 13 bytes. Read
/// the drift for its order of magnitude, not its sign; kilobytes mean something
/// changed.
pub const WASM_RAW_ACHIEVED_BYTES: u64 = 4_009_656;

/// The ceiling `cargo xtask validate` enforces.
///
/// Decimal 5 MB, not 5 MiB: #1750 accepts additional feature headroom.
/// See ADR-0106's amendment for the tradeoff; the achieved baseline stays historical.
pub const WASM_RAW_CEILING_BYTES: u64 = 5_000_000;

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
        "raw manifest-selected WASM identity artifact is {} bytes, over the {} byte ceiling by {}.\n\
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
        assert_eq!(WASM_RAW_CEILING_BYTES, 5_000_000);
        assert!(!check(5_000_000, WASM_RAW_CEILING_BYTES).over);
    }

    #[test]
    fn over_ceiling_fails() {
        assert!(check(5_000_001, WASM_RAW_CEILING_BYTES).over);
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
    /// after #1625 added the Manage Posts workspace.
    /// These historical measurements are not sizes of the current bundle.
    const NO_WASM_OPT_BYTES: u64 = 5_350_591;
    const O2_LEVEL_BYTES: u64 = 4_252_496;
    const OS_LEVEL_BYTES: u64 = 4_191_207;

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
    fn historical_weaker_optimisation_sizes_fit_the_approved_budget() {
        // ADR-0106's #1750 amendment accepts this lost size-only guard;
        // the production optimiser remains -Oz.
        assert!(
            !check(OS_LEVEL_BYTES, WASM_RAW_CEILING_BYTES).over,
            "the historical -Os size fits the approved 5 MB ceiling"
        );
        assert!(
            !check(O2_LEVEL_BYTES, WASM_RAW_CEILING_BYTES).over,
            "the historical -O2 size fits the approved 5 MB ceiling"
        );
    }

    #[test]
    fn the_pre_836_bundle_would_fail_todays_budget() {
        assert!(
            check(NO_WASM_OPT_BYTES, WASM_RAW_CEILING_BYTES).over,
            "the historical pre-#836 size must fail the gate"
        );
    }
}

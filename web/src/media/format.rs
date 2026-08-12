//! Human-readable byte-size display for the media vertical — the quota/usage and
//! per-item size labels the media UI paints. Pure and host-tested (ADR-0070's extra
//! leaf), even though its only caller is the wasm-only `component`.

/// `numerator / denominator` to one decimal place, rounded **half-to-even**, in
/// integer arithmetic.
///
/// Half-to-even is not a stylistic choice — it is the rule Rust's `{:.1}` applies
/// to an `f64`, and ties genuinely occur here. 1280 bytes is exactly 1.25 KB, which
/// `{:.1}` renders `"1.2"`, not `"1.3"`; 1792 bytes is 1.75 KB and renders `"1.8"`.
/// (The intuition that a power-of-two divisor cannot produce a `.x5` tie is wrong:
/// `1280/1024 = 5/4` lands exactly on one.) Rounding half-up here would diverge from
/// the previous float implementation on the very first tie.
///
/// `i128` throughout so `numerator * 10` cannot overflow for an `i64` input, and so
/// there is no `as f64` — below 2^53 this reproduces the float output exactly, and
/// above it, where the `as f64` conversion was itself lossy, this is the more
/// accurate of the two.
fn one_decimal(numerator: i64, denominator: i64) -> String {
    debug_assert!(denominator > 0, "callers divide by a positive unit size");
    debug_assert!(numerator >= 0, "negative byte counts take the `B` arm");

    let tenths = round_tenths(i128::from(numerator) * 10, i128::from(denominator));
    format_tenths(tenths)
}

/// `num / den` rounded **half-to-even** — the rule Rust's `{:.1}` applies to an
/// `f64`, and the one thing in this module that must not drift between its two
/// callers. Each of them pre-scales `num` for the precision it wants (`×10` for a
/// unit quotient, `×1000` for a percentage), so only the rounding lives here.
fn round_tenths(num: i128, den: i128) -> i128 {
    let mut tenths = num / den;
    let doubled_remainder = (num % den) * 2;
    if doubled_remainder > den || (doubled_remainder == den && tenths % 2 != 0) {
        tenths += 1;
    }
    tenths
}

/// A tenths count as a one-decimal string: `125` → `"12.5"`.
fn format_tenths(tenths: i128) -> String {
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Formats a byte count as a human-readable size (`B` / `KB` / `MB` / `GB`, one
/// decimal). Shared display formatter, host-tested here.
#[must_use]
pub fn format_bytes(bytes: impl Into<i64>) -> String {
    const KB: i64 = 1_024;
    const MB: i64 = 1_024 * KB;
    const GB: i64 = 1_024 * MB;

    // Generic over the byte-ish newtypes (`ByteSize`, `MaxFileSize`, `UserQuota` — each
    // `Into<i64>` via `NumNewtype`) as well as a bare `i64`, so call sites pass the typed
    // value without spelling `.value()`.
    let bytes: i64 = bytes.into();

    if bytes >= GB {
        format!("{} GB", one_decimal(bytes, GB))
    } else if bytes >= MB {
        format!("{} MB", one_decimal(bytes, MB))
    } else if bytes >= KB {
        format!("{} KB", one_decimal(bytes, KB))
    } else {
        format!("{bytes} B")
    }
}

/// The storage-usage percentage as a one-decimal string, clamped to `0.0..=100.0`.
///
/// Lifted out of the wasm-only media component so it can be host-tested at all
/// (`media/component.rs` is never host-compiled and `cargo llvm-cov` cannot
/// instrument wasm — ADR-0055). Returning the formatted string rather than a number
/// also removes a branch from the component's `Suspend` body, which sits at the
/// `thin-components` budget.
///
/// Integer math, deliberately not `used as f64 / quota as f64 * 100.0`: two
/// successive binary roundings against an arbitrary (non-power-of-two) `quota`
/// make the last digit an artifact of the rounding path rather than of the true
/// ratio. This rounds the true ratio once, half-to-even (#301).
///
/// It also clamps at **both** ends. A negative `used` is not reachable from a
/// byte count today, but the type permits one, and `0.0` is the honest floor for
/// a usage bar (an unclamped expression would produce a negative width).
#[must_use]
pub fn storage_usage_percent(used: i64, quota: i64) -> String {
    if quota <= 0 || used <= 0 {
        return "0.0".to_string();
    }
    // pct * 10 == used * 1000 / quota.
    let tenths = round_tenths(i128::from(used) * 1000, i128::from(quota)).min(1000);
    format_tenths(tenths)
}

#[cfg(test)]
mod tests {
    use super::{format_bytes, storage_usage_percent};

    #[test]
    fn format_bytes_displays_bytes_below_kb() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_displays_kb_range() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn format_bytes_displays_mb_range() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 2), "2.0 MB");
    }

    #[test]
    fn format_bytes_displays_gb_range() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    /// The rounding rule, pinned by construction rather than assumed.
    ///
    /// A power-of-two divisor *can* land exactly on a one-decimal tie — 1280/1024 is
    /// 5/4 — and Rust's `{:.1}` breaks such ties to even. Half-up rounding would
    /// render these "1.3" / "2.3" and silently diverge from the float implementation
    /// this replaced.
    #[test]
    fn exact_ties_round_half_to_even() {
        assert_eq!(format_bytes(1280), "1.2 KB", "1.25 rounds down to even");
        assert_eq!(format_bytes(1792), "1.8 KB", "1.75 rounds up to even");
        assert_eq!(format_bytes(2304), "2.2 KB", "2.25 rounds down to even");
        assert_eq!(format_bytes(2816), "2.8 KB", "2.75 rounds up to even");
    }

    #[test]
    fn format_bytes_at_each_unit_boundary() {
        const KB: i64 = 1_024;
        const MB: i64 = 1_024 * KB;
        const GB: i64 = 1_024 * MB;

        assert_eq!(format_bytes(KB - 1), "1023 B");
        assert_eq!(format_bytes(KB), "1.0 KB");
        assert_eq!(format_bytes(KB + 1), "1.0 KB");
        assert_eq!(format_bytes(MB - 1), "1024.0 KB");
        assert_eq!(format_bytes(MB), "1.0 MB");
        assert_eq!(format_bytes(MB + 1), "1.0 MB");
        assert_eq!(format_bytes(GB - 1), "1024.0 MB");
        assert_eq!(format_bytes(GB), "1.0 GB");
        assert_eq!(format_bytes(GB + 1), "1.0 GB");
    }

    /// `format_bytes` takes `impl Into<i64>`, so a negative is reachable and must
    /// take the byte arm rather than dividing.
    #[test]
    fn format_bytes_passes_negatives_through_as_bytes() {
        assert_eq!(format_bytes(-1), "-1 B");
        assert_eq!(format_bytes(-4096), "-4096 B");
    }

    /// 2^53 is f64's exactness boundary — where an `as f64` conversion would stop
    /// being exact. Pinned here because `i64::MAX` — the obvious "large value"
    /// test — coincidentally agrees between integer and float math and would hide
    /// a wrong result across the entire petabyte band.
    #[test]
    fn format_bytes_is_exact_past_the_f64_mantissa() {
        const GB: i64 = 1_024 * 1_024 * 1_024;
        const TWO_POW_53: i64 = 1 << 53;

        assert_eq!(format_bytes(TWO_POW_53), "8388608.0 GB");
        // One byte past the mantissa bound: exact in integers, not in f64.
        assert_eq!(format_bytes(TWO_POW_53 + 1), "8388608.0 GB");
        assert_eq!(format_bytes(i64::MAX), "8589934592.0 GB");
        assert_eq!(format_bytes(i64::MAX - GB), "8589934591.0 GB");
    }

    #[test]
    fn storage_usage_percent_covers_the_degenerate_and_clamped_cases() {
        assert_eq!(storage_usage_percent(0, 0), "0.0", "no quota");
        assert_eq!(storage_usage_percent(5, 0), "0.0", "no quota, some usage");
        assert_eq!(storage_usage_percent(0, 100), "0.0", "no usage");
        assert_eq!(storage_usage_percent(100, 100), "100.0", "exactly full");
        assert_eq!(
            storage_usage_percent(-5, 100),
            "0.0",
            "clamped under zero — the float version clamped only the top, and would \
             have produced a negative bar width"
        );
        assert_eq!(
            storage_usage_percent(250, 100),
            "100.0",
            "clamped over quota"
        );
    }

    #[test]
    fn storage_usage_percent_rounds_half_to_even() {
        // 1/16 = 6.25% exactly — a tie, rounding down to even.
        assert_eq!(storage_usage_percent(1, 16), "6.2");
        // 3/16 = 18.75% exactly — a tie, rounding up to even.
        assert_eq!(storage_usage_percent(3, 16), "18.8");
        assert_eq!(storage_usage_percent(1, 3), "33.3");
        assert_eq!(storage_usage_percent(2, 3), "66.7");
    }
}

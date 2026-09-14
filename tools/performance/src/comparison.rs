use crate::model::Summary;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Regression {
    None,
    AtLeastTwentyPercent,
    ZeroBaselineIncrease,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Comparison {
    pub median: Regression,
    pub p95: Regression,
}

#[must_use]
/// Timing deltas are evidence only; callers report these classifications without turning them into failure.
pub fn compare_summary(baseline: &Summary, candidate: &Summary) -> Comparison {
    Comparison {
        median: classify(baseline.median_us, candidate.median_us),
        p95: classify(baseline.p95_us, candidate.p95_us),
    }
}

fn classify(baseline: u64, candidate: u64) -> Regression {
    if baseline == 0 {
        return if candidate == 0 {
            Regression::None
        } else {
            Regression::ZeroBaselineIncrease
        };
    }
    let threshold = baseline / 5 + u64::from(!baseline.is_multiple_of(5));
    if candidate > baseline && candidate - baseline >= threshold {
        Regression::AtLeastTwentyPercent
    } else {
        Regression::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(median_us: u64, p95_us: u64) -> Summary {
        Summary {
            sample_count: 1,
            minimum_us: 0,
            maximum_us: 0,
            mean_us: 0,
            median_us,
            p95_us,
        }
    }

    #[test]
    fn classifies_twenty_percent_boundary_without_floats() {
        let baseline = summary(100, 100);
        assert_eq!(
            compare_summary(&baseline, &summary(119, 119)),
            Comparison {
                median: Regression::None,
                p95: Regression::None
            }
        );
        assert_eq!(
            compare_summary(&baseline, &summary(120, 120)),
            Comparison {
                median: Regression::AtLeastTwentyPercent,
                p95: Regression::AtLeastTwentyPercent
            }
        );
        assert_eq!(
            compare_summary(&baseline, &summary(121, 121)),
            Comparison {
                median: Regression::AtLeastTwentyPercent,
                p95: Regression::AtLeastTwentyPercent
            }
        );
    }

    #[test]
    fn zero_baselines_are_safe_and_visible() {
        assert_eq!(
            compare_summary(&summary(0, 0), &summary(0, 0)),
            Comparison {
                median: Regression::None,
                p95: Regression::None
            }
        );
        assert_eq!(
            compare_summary(&summary(0, 0), &summary(1, 1)),
            Comparison {
                median: Regression::ZeroBaselineIncrease,
                p95: Regression::ZeroBaselineIncrease
            }
        );
    }
}

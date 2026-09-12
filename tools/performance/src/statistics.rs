use thiserror::Error;

use crate::model::{RawSample, Summary};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum StatisticsError {
    #[error("a workload needs at least one sample")]
    EmptySamples,
    #[error("the arithmetic mean exceeds the persisted duration range")]
    MeanOutOfRange,
}

/// Computes order statistics in integer microseconds, retaining reproducible artifact identity.
///
/// # Errors
///
/// Returns [`StatisticsError::EmptySamples`] for an empty input and
/// [`StatisticsError::MeanOutOfRange`] if the computed mean cannot be persisted.
pub fn summarize(samples: &[RawSample]) -> Result<Summary, StatisticsError> {
    if samples.is_empty() {
        return Err(StatisticsError::EmptySamples);
    }
    let mut durations = samples
        .iter()
        .map(|sample| sample.duration_us)
        .collect::<Vec<_>>();
    durations.sort_unstable();
    let count = durations.len();
    let total = durations
        .iter()
        .map(|duration| u128::from(*duration))
        .sum::<u128>();
    let mean_us =
        u64::try_from(total / count as u128).map_err(|_| StatisticsError::MeanOutOfRange)?;
    Ok(Summary {
        sample_count: count
            .try_into()
            .map_err(|_| StatisticsError::EmptySamples)?,
        minimum_us: durations[0],
        maximum_us: durations[count - 1],
        mean_us,
        median_us: durations[(count - 1) / 2]
            + (durations[count / 2] - durations[(count - 1) / 2]) / 2,
        p95_us: durations[(count * 95).div_ceil(100) - 1],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_midpoint_median_and_nearest_rank_p95() {
        let samples = [10, 20, 40, 100].map(|duration_us| RawSample { duration_us });
        assert_eq!(
            summarize(&samples).unwrap(),
            Summary {
                sample_count: 4,
                minimum_us: 10,
                maximum_us: 100,
                mean_us: 42,
                median_us: 30,
                p95_us: 100
            }
        );
    }

    #[test]
    fn rejects_missing_samples() {
        assert_eq!(summarize(&[]), Err(StatisticsError::EmptySamples));
    }
}

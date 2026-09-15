//! Versioned two-worker coverage evidence and exact population reconciliation.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::status::{ProcessOutcome, TestCensus};

pub const WORKER_EVIDENCE_VERSION: u32 = 1;
const WORKER_COUNT: u8 = 2;

/// The only partitioning strategies that can appear in evidence or the CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExperimentStrategy {
    Baseline,
    Slice,
    Hash,
    Backend,
}

impl ExperimentStrategy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Slice => "slice",
            Self::Hash => "hash",
            Self::Backend => "backend",
        }
    }
}

impl fmt::Display for ExperimentStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExperimentStrategy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "slice" => Ok(Self::Slice),
            "hash" => Ok(Self::Hash),
            "backend" => Ok(Self::Backend),
            _ => Err(format!(
                "unknown coverage experiment strategy `{value}`; expected baseline, slice, hash, or backend"
            )),
        }
    }
}

/// The CPU allocation policy for a two-worker coverage experiment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkerConcurrencyPolicy {
    Independent,
    Fixed,
}

impl WorkerConcurrencyPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::Fixed => "fixed",
        }
    }
}

impl fmt::Display for WorkerConcurrencyPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkerConcurrencyPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "independent" => Ok(Self::Independent),
            "fixed" => Ok(Self::Fixed),
            _ => Err(format!(
                "unknown coverage worker concurrency `{value}`; expected independent or fixed"
            )),
        }
    }
}

/// The fixed position of a worker within one partitioning strategy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerPartition {
    pub strategy: ExperimentStrategy,
    pub index: u8,
    pub total: u8,
}

/// Terminal evidence emitted by one coverage worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerEvidence {
    pub version: u32,
    pub partition: WorkerPartition,
    pub outcome: ProcessOutcome,
    pub census: TestCensus,
    pub profile_artifacts: Vec<String>,
    pub duration_ms: u128,
    pub diagnostics: Vec<String>,
}

impl WorkerEvidence {
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("serialize worker evidence")
        )
    }

    pub fn from_json(input: &str) -> Result<Self> {
        let evidence: Self = serde_json::from_str(input)?;
        evidence.validate()?;
        Ok(evidence)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != WORKER_EVIDENCE_VERSION {
            bail!("unsupported worker evidence version {}", self.version);
        }
        validate_partition(&self.partition)?;
        if matches!(self.outcome, ProcessOutcome::NotRun) {
            bail!("worker evidence is not terminal");
        }
        validate_worker_census(&self.census)?;
        if self.outcome.is_success() && self.profile_artifacts.is_empty() {
            bail!("successful worker is missing a profile artifact");
        }
        if self.diagnostics.is_empty() {
            bail!("missing diagnostic");
        }
        validate_nonempty_distinct("profile artifact", &self.profile_artifacts)?;
        validate_nonempty_distinct("diagnostic", &self.diagnostics)?;
        Ok(())
    }
}

/// Whether the worker evidence was completely reconciled to the expected census.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Reconciliation {
    Reconciled,
    Error { detail: String },
}

/// Fully retained evidence from both workers, including failed reconciliation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateEvidence {
    pub strategy: ExperimentStrategy,
    pub population: TestCensus,
    pub workers: Vec<WorkerEvidence>,
    pub reconciliation: Reconciliation,
}

/// Preserve both terminal records even when their evidence cannot reconcile.
pub fn aggregate_terminal(
    strategy: ExperimentStrategy,
    expected: &TestCensus,
    workers: &[WorkerEvidence],
) -> AggregateEvidence {
    match aggregate_workers(expected, workers) {
        Ok(aggregate) => aggregate,
        Err(error) => AggregateEvidence {
            strategy,
            population: available_population(workers),
            workers: workers.to_vec(),
            reconciliation: Reconciliation::Error {
                detail: error.to_string(),
            },
        },
    }
}

/// Validate and merge exactly two terminal worker records against one census.
pub fn aggregate_workers(
    expected: &TestCensus,
    workers: &[WorkerEvidence],
) -> Result<AggregateEvidence> {
    validate_expected_census(expected)?;
    if workers.len() != usize::from(WORKER_COUNT) {
        bail!("coverage aggregation requires exactly two workers");
    }

    let mut indexes = BTreeSet::new();
    let mut strategy = None;
    let mut profile_artifacts = BTreeSet::new();
    let mut diagnostics = BTreeSet::new();
    let mut executed = BTreeSet::new();
    let mut ignored = BTreeSet::new();
    let mut failed = BTreeSet::new();

    for worker in workers {
        worker.validate()?;
        if worker.profile_artifacts.is_empty() {
            bail!(
                "worker {} has no profile artifact for the merged report",
                worker.partition.index
            );
        }
        if !indexes.insert(worker.partition.index) {
            bail!("duplicate worker index");
        }
        match strategy {
            Some(existing) if existing != worker.partition.strategy => {
                bail!("worker strategies do not match");
            }
            None => strategy = Some(worker.partition.strategy),
            _ => {}
        }
        insert_distinct(
            "profile artifact",
            &mut profile_artifacts,
            &worker.profile_artifacts,
        )?;
        insert_distinct("diagnostic", &mut diagnostics, &worker.diagnostics)?;
        insert_population(&worker.census, &mut executed, &mut ignored, &mut failed)?;
    }

    if indexes != BTreeSet::from([1, 2]) {
        bail!("worker indexes must be exactly 1 and 2");
    }

    let actual = TestCensus {
        expected: Vec::new(),
        executed: executed.into_iter().collect(),
        ignored: ignored.into_iter().collect(),
        failed: failed.into_iter().collect(),
    };
    reconcile_population(expected, &actual)?;

    Ok(AggregateEvidence {
        strategy: strategy.expect("two workers have a strategy"),
        population: actual,
        workers: workers.to_vec(),
        reconciliation: Reconciliation::Reconciled,
    })
}

fn available_population(workers: &[WorkerEvidence]) -> TestCensus {
    let mut population = TestCensus::default();
    for worker in workers {
        population
            .executed
            .extend(worker.census.executed.iter().cloned());
        population
            .ignored
            .extend(worker.census.ignored.iter().cloned());
        population
            .failed
            .extend(worker.census.failed.iter().cloned());
    }
    population.executed.sort();
    population.ignored.sort();
    population.failed.sort();
    population
}

fn validate_partition(partition: &WorkerPartition) -> Result<()> {
    if partition.total != WORKER_COUNT || !(1..=WORKER_COUNT).contains(&partition.index) {
        bail!("worker partition must be one of two workers");
    }
    Ok(())
}

fn validate_expected_census(census: &TestCensus) -> Result<()> {
    if census.expected.is_empty() {
        bail!("empty expected test census");
    }
    if !census.executed.is_empty() || !census.failed.is_empty() {
        bail!("expected test census contains terminal results");
    }
    validate_nonempty_distinct("expected test identity", &census.expected)?;
    validate_nonempty_distinct("ignored test identity", &census.ignored)?;
    let expected = census.expected.iter().collect::<BTreeSet<_>>();
    if !census
        .ignored
        .iter()
        .all(|identity| expected.contains(identity))
    {
        bail!("expected ignored test is absent from census");
    }
    Ok(())
}

fn validate_worker_census(census: &TestCensus) -> Result<()> {
    if !census.expected.is_empty() {
        bail!("worker JUnit census contains expected tests");
    }
    validate_nonempty_distinct("executed test identity", &census.executed)?;
    validate_nonempty_distinct("ignored test identity", &census.ignored)?;
    validate_nonempty_distinct("failed test identity", &census.failed)?;
    let executed = census.executed.iter().collect::<BTreeSet<_>>();
    let ignored = census.ignored.iter().collect::<BTreeSet<_>>();
    if !executed.is_disjoint(&ignored) {
        bail!("worker JUnit census duplicates a test identity");
    }
    if !census
        .failed
        .iter()
        .all(|identity| executed.contains(identity))
    {
        bail!("worker failed test is absent from executed population");
    }
    Ok(())
}

fn validate_nonempty_distinct(kind: &str, values: &[String]) -> Result<()> {
    if values.iter().any(String::is_empty) {
        bail!("empty {kind}");
    }
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        bail!("duplicate {kind}");
    }
    Ok(())
}

fn insert_distinct(kind: &str, seen: &mut BTreeSet<String>, values: &[String]) -> Result<()> {
    for value in values {
        if !seen.insert(value.clone()) {
            bail!("duplicate {kind}");
        }
    }
    Ok(())
}

fn insert_population(
    census: &TestCensus,
    executed: &mut BTreeSet<String>,
    ignored: &mut BTreeSet<String>,
    failed: &mut BTreeSet<String>,
) -> Result<()> {
    for identity in &census.executed {
        if !executed.insert(identity.clone()) || ignored.contains(identity) {
            bail!("duplicate executed test identity");
        }
    }
    for identity in &census.ignored {
        if !ignored.insert(identity.clone()) || executed.contains(identity) {
            bail!("duplicate ignored test identity");
        }
    }
    failed.extend(census.failed.iter().cloned());
    Ok(())
}

fn reconcile_population(expected: &TestCensus, actual: &TestCensus) -> Result<()> {
    let expected_set = expected.expected.iter().collect::<BTreeSet<_>>();
    let expected_ignored = expected.ignored.iter().collect::<BTreeSet<_>>();
    let executed = actual.executed.iter().collect::<BTreeSet<_>>();
    let ignored = actual.ignored.iter().collect::<BTreeSet<_>>();
    let actual_set = executed.union(&ignored).copied().collect::<BTreeSet<_>>();
    if expected_set != actual_set || expected_ignored != ignored {
        bail!("worker populations do not reconcile to the expected census");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> TestCensus {
        TestCensus {
            expected: vec![
                "bin::first".into(),
                "bin::ignored".into(),
                "bin::second".into(),
            ],
            ignored: vec!["bin::ignored".into()],
            ..TestCensus::default()
        }
    }

    fn worker(index: u8, census: TestCensus) -> WorkerEvidence {
        WorkerEvidence {
            version: WORKER_EVIDENCE_VERSION,
            partition: WorkerPartition {
                strategy: ExperimentStrategy::Slice,
                index,
                total: 2,
            },
            outcome: ProcessOutcome::Success,
            census,
            profile_artifacts: vec![format!("profiles/worker-{index}.profraw")],
            duration_ms: 12,
            diagnostics: vec![format!("logs/worker-{index}.log")],
        }
    }

    fn first_worker() -> WorkerEvidence {
        worker(
            1,
            TestCensus {
                executed: vec!["bin::first".into()],
                ..TestCensus::default()
            },
        )
    }

    fn second_worker() -> WorkerEvidence {
        worker(
            2,
            TestCensus {
                executed: vec!["bin::second".into()],
                ignored: vec!["bin::ignored".into()],
                ..TestCensus::default()
            },
        )
    }

    #[test]
    fn parses_only_complete_versioned_terminal_worker_evidence() {
        let record = first_worker();
        assert_eq!(
            WorkerEvidence::from_json(&record.to_json()).unwrap(),
            record
        );

        let mut unsupported_version = record.clone();
        unsupported_version.version = WORKER_EVIDENCE_VERSION + 1;
        assert!(WorkerEvidence::from_json(&unsupported_version.to_json()).is_err());

        let mut empty_diagnostic_reference = record.clone();
        empty_diagnostic_reference.diagnostics = vec![String::new()];
        assert!(WorkerEvidence::from_json(&empty_diagnostic_reference.to_json()).is_err());

        let mut non_terminal = record;
        non_terminal.outcome = ProcessOutcome::NotRun;
        assert!(WorkerEvidence::from_json(&non_terminal.to_json()).is_err());
    }

    #[test]
    fn aggregates_the_exact_two_worker_union() {
        let aggregate = aggregate_workers(&expected(), &[first_worker(), second_worker()]).unwrap();
        assert_eq!(aggregate.strategy, ExperimentStrategy::Slice);
        assert_eq!(
            aggregate.population.executed,
            vec!["bin::first".to_owned(), "bin::second".to_owned()]
        );
        assert_eq!(
            aggregate.population.ignored,
            vec!["bin::ignored".to_owned()]
        );
        assert_eq!(aggregate.workers[0].duration_ms, 12);
        assert_eq!(
            aggregate.workers[1].profile_artifacts,
            vec!["profiles/worker-2.profraw".to_owned()]
        );
    }

    #[test]
    fn rejects_invalid_partitions_and_worker_counts() {
        let mut duplicate = second_worker();
        duplicate.partition.index = 1;
        assert!(aggregate_workers(&expected(), &[first_worker(), duplicate]).is_err());
        assert!(aggregate_workers(&expected(), &[first_worker()]).is_err());

        let mut wrong_total = second_worker();
        wrong_total.partition.total = 3;
        assert!(aggregate_workers(&expected(), &[first_worker(), wrong_total]).is_err());

        let mut mismatched_strategy = second_worker();
        mismatched_strategy.partition.strategy = ExperimentStrategy::Hash;
        assert!(aggregate_workers(&expected(), &[first_worker(), mismatched_strategy]).is_err());
    }

    #[test]
    fn rejects_duplicate_missing_or_inconsistently_ignored_population() {
        let mut duplicate_executed = second_worker();
        duplicate_executed.census.executed = vec!["bin::first".into()];
        assert!(aggregate_workers(&expected(), &[first_worker(), duplicate_executed]).is_err());

        let mut duplicate_ignored = first_worker();
        duplicate_ignored.census.ignored = vec!["bin::ignored".into()];
        assert!(aggregate_workers(&expected(), &[duplicate_ignored, second_worker()]).is_err());

        let missing = worker(2, TestCensus::default());
        assert!(aggregate_workers(&expected(), &[first_worker(), missing]).is_err());

        let mut wrong_ignored = second_worker();
        wrong_ignored.census.ignored = Vec::new();
        wrong_ignored.census.executed.push("bin::ignored".into());
        assert!(aggregate_workers(&expected(), &[first_worker(), wrong_ignored]).is_err());
    }

    #[test]
    fn rejects_missing_or_duplicate_artifacts_and_invalid_failure_diagnostics() {
        let mut missing_profile = first_worker();
        missing_profile.profile_artifacts.clear();
        assert!(aggregate_workers(&expected(), &[missing_profile, second_worker()]).is_err());

        let mut duplicate_profile = second_worker();
        duplicate_profile.profile_artifacts = first_worker().profile_artifacts;
        assert!(aggregate_workers(&expected(), &[first_worker(), duplicate_profile]).is_err());

        let mut duplicate_diagnostic = second_worker();
        duplicate_diagnostic.diagnostics = first_worker().diagnostics;
        assert!(aggregate_workers(&expected(), &[first_worker(), duplicate_diagnostic]).is_err());

        let mut failed_without_diagnostics = first_worker();
        failed_without_diagnostics.outcome = ProcessOutcome::ExitCode { exit_code: 1 };
        failed_without_diagnostics.diagnostics.clear();
        assert!(
            aggregate_workers(&expected(), &[failed_without_diagnostics, second_worker()]).is_err()
        );
    }

    #[test]
    fn retains_a_single_worker_test_failure() {
        let mut failing = first_worker();
        failing.outcome = ProcessOutcome::ExitCode { exit_code: 1 };
        failing.census.failed = vec!["bin::first".into()];
        let aggregate =
            aggregate_workers(&expected(), &[failing.clone(), second_worker()]).unwrap();
        assert_eq!(aggregate.population.failed, vec!["bin::first".to_owned()]);
        assert_eq!(aggregate.workers[0].outcome, failing.outcome);
    }

    #[test]
    fn strategy_rejects_unknown_cli_and_wire_values() {
        assert_eq!(
            "slice".parse::<ExperimentStrategy>(),
            Ok(ExperimentStrategy::Slice)
        );
        assert!("not-a-strategy".parse::<ExperimentStrategy>().is_err());
        assert!(serde_json::from_str::<ExperimentStrategy>("\"not-a-strategy\"").is_err());
    }

    #[test]
    fn terminal_aggregate_retains_distinct_dual_failures_when_profiles_are_empty() {
        let mut first = first_worker();
        first.outcome = ProcessOutcome::SpawnError {
            spawn_error: "cannot spawn nextest".into(),
        };
        first.profile_artifacts.clear();
        first.diagnostics = vec!["logs/first-spawn.log".into()];

        let mut second = second_worker();
        second.outcome = ProcessOutcome::Signal;
        second.profile_artifacts.clear();
        second.diagnostics = vec!["logs/second-signal.log".into()];

        let aggregate = aggregate_terminal(
            ExperimentStrategy::Slice,
            &expected(),
            &[first.clone(), second.clone()],
        );
        assert_eq!(aggregate.workers, vec![first, second]);
        assert_eq!(
            aggregate.population.executed,
            vec!["bin::first".to_owned(), "bin::second".to_owned()]
        );
        assert!(matches!(
            aggregate.reconciliation,
            Reconciliation::Error { .. }
        ));
    }

    #[test]
    fn retains_distinct_terminal_evidence_from_simultaneous_failures() {
        let mut first = first_worker();
        first.outcome = ProcessOutcome::ExitCode { exit_code: 1 };
        first.census.failed = vec!["bin::first".into()];
        first.diagnostics = vec!["logs/first-failure.log".into()];

        let mut second = second_worker();
        second.outcome = ProcessOutcome::Signal;
        second.diagnostics = vec!["logs/second-signal.log".into()];

        let aggregate = aggregate_workers(&expected(), &[first.clone(), second.clone()]).unwrap();
        assert_eq!(aggregate.workers, vec![first, second]);
        assert_eq!(aggregate.population.failed, vec!["bin::first".to_owned()]);
    }
}

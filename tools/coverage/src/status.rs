//! Versioned, fail-closed coverage producer evidence.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use quick_xml::Reader;
use quick_xml::encoding::Decoder;
use quick_xml::events::Event;
use quick_xml::events::attributes::Attributes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const COVERAGE_STATUS_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusCategory {
    TestsOk,
    TestFailure,
    Infra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredStage {
    WorkspaceResolution,
    ProfileCleanup,
    TestCensus,
    InstrumentedTestRun,
    PopulationReconciliation,
    TextReport,
    LcovReport,
    CrapReport,
}

impl RequiredStage {
    pub const ALL: [Self; 8] = [
        Self::WorkspaceResolution,
        Self::ProfileCleanup,
        Self::TestCensus,
        Self::InstrumentedTestRun,
        Self::PopulationReconciliation,
        Self::TextReport,
        Self::LcovReport,
        Self::CrapReport,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceResolution => "workspace-resolution",
            Self::ProfileCleanup => "profile-cleanup",
            Self::TestCensus => "test-census",
            Self::InstrumentedTestRun => "instrumented-test-run",
            Self::PopulationReconciliation => "population-reconciliation",
            Self::TextReport => "text-report",
            Self::LcovReport => "lcov-report",
            Self::CrapReport => "crap-report",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessOutcome {
    Success,
    NotRun,
    ExitCode { exit_code: i32 },
    Signal,
    SpawnError { spawn_error: String },
    EvidenceError { evidence_error: String },
}

impl ProcessOutcome {
    pub const fn success() -> Self {
        Self::Success
    }

    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    pub const fn is_failure(&self) -> bool {
        !self.is_success()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageResult {
    pub stage: RequiredStage,
    pub outcome: ProcessOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Population {
    pub expected: usize,
    pub executed: usize,
    pub ignored: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageStatus {
    pub version: u32,
    pub category: StatusCategory,
    pub stages: Vec<StageResult>,
    pub population: Population,
    #[serde(default)]
    pub failed_tests: Vec<String>,
    #[serde(default)]
    pub missing_tests: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infra_detail: Option<String>,
}

/// Stable JUnit testcase identity and its terminal classification.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestCensus {
    pub expected: Vec<String>,
    pub executed: Vec<String>,
    pub ignored: Vec<String>,
    pub failed: Vec<String>,
}

fn test_identity(binary_id: &str, test_name: &str) -> Result<String> {
    if binary_id.is_empty() || test_name.is_empty() {
        bail!("empty test identity");
    }
    Ok(format!("{binary_id}::{test_name}"))
}

/// Parse the stable `cargo nextest list --message-format json` summary.
pub fn parse_nextest_census(input: &str) -> Result<TestCensus> {
    let value: Value = serde_json::from_str(input)?;
    let declared_count = value["test-count"].as_u64().context("missing test-count")?;
    let suites = value["rust-suites"]
        .as_object()
        .context("missing rust-suites")?;
    let mut census = TestCensus::default();
    for suite in suites.values() {
        let binary = suite["binary-id"].as_str().context("missing binary-id")?;
        let tests = suite["testcases"]
            .as_object()
            .context("missing testcases")?;
        for (name, testcase) in tests {
            let test = test_identity(binary, name)?;
            let ignored = testcase["ignored"].as_bool().context("missing ignored")?;
            let filter_match = testcase["filter-match"]
                .as_object()
                .context("missing filter-match")?;
            let filter_status = filter_match["status"]
                .as_str()
                .context("missing filter-match status")?;
            if census.expected.contains(&test) {
                bail!("duplicate test identity");
            }
            match (
                ignored,
                filter_status,
                filter_match.get("reason").and_then(Value::as_str),
            ) {
                (false, "matches", _) => {}
                (true, "mismatch", Some("ignored")) => census.ignored.push(test.clone()),
                _ => bail!("census contains a filtered or inconsistent test"),
            }
            census.expected.push(test);
        }
    }
    if census.expected.len() != usize::try_from(declared_count).context("test-count overflows")? {
        bail!("test-count does not match testcases");
    }
    if census.expected.is_empty() {
        bail!("empty test census");
    }
    Ok(census)
}

#[derive(Debug)]
struct JunitTestcase {
    identity: String,
    ignored: bool,
    failed: bool,
}

impl JunitTestcase {
    fn new(attributes: Attributes<'_>, decoder: Decoder) -> Result<Self> {
        let mut classname = None;
        let mut name = None;
        for attribute in attributes {
            let attribute = attribute?;
            let value = attribute.decode_and_unescape_value(decoder)?.into_owned();
            match attribute.key.as_ref() {
                b"classname" => classname = Some(value),
                b"name" => name = Some(value),
                _ => {}
            }
        }
        Ok(Self {
            identity: test_identity(
                classname.as_deref().context("missing classname")?,
                name.as_deref().context("missing testcase name")?,
            )?,
            ignored: false,
            failed: false,
        })
    }
}

/// Parse JUnit terminal testcase results without treating report prose as evidence.
pub fn parse_junit_census(input: &str) -> Result<TestCensus> {
    let mut reader = Reader::from_reader(input.as_bytes());
    let mut buffer = Vec::new();
    let mut census = TestCensus::default();
    let mut testcase = None;

    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(event) if event.name().as_ref() == b"testcase" => {
                if testcase.is_some() {
                    bail!("nested testcase");
                }
                testcase = Some(JunitTestcase::new(event.attributes(), reader.decoder())?);
            }
            Event::Empty(event) if event.name().as_ref() == b"testcase" => {
                finish_junit_testcase(
                    &mut census,
                    JunitTestcase::new(event.attributes(), reader.decoder())?,
                )?;
            }
            Event::Start(event) | Event::Empty(event) if event.name().as_ref() == b"skipped" => {
                testcase
                    .as_mut()
                    .context("skipped outside testcase")?
                    .ignored = true;
            }
            Event::Start(event) | Event::Empty(event)
                if matches!(event.name().as_ref(), b"failure" | b"error") =>
            {
                testcase
                    .as_mut()
                    .context("failure outside testcase")?
                    .failed = true;
            }
            Event::End(event) if event.name().as_ref() == b"testcase" => {
                finish_junit_testcase(
                    &mut census,
                    testcase.take().context("testcase end without start")?,
                )?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if testcase.is_some() {
        bail!("unterminated testcase");
    }
    Ok(census)
}

fn finish_junit_testcase(census: &mut TestCensus, testcase: JunitTestcase) -> Result<()> {
    if census.executed.contains(&testcase.identity) || census.ignored.contains(&testcase.identity) {
        bail!("duplicate test identity");
    }
    if testcase.ignored && testcase.failed {
        bail!("testcase cannot be both skipped and failed");
    }
    if testcase.ignored {
        census.ignored.push(testcase.identity);
    } else {
        if testcase.failed {
            census.failed.push(testcase.identity.clone());
        }
        census.executed.push(testcase.identity);
    }
    Ok(())
}

/// Reconcile exact expected and terminal JUnit testcase identity sets.
pub fn reconcile_test_census(expected: &TestCensus, actual: &TestCensus) -> Result<Vec<String>> {
    let expected_set: BTreeSet<_> = expected.expected.iter().cloned().collect();
    let ignored_set: BTreeSet<_> = expected.ignored.iter().cloned().collect();
    let executed_set: BTreeSet<_> = actual.executed.iter().cloned().collect();
    let actual_ignored_set: BTreeSet<_> = actual.ignored.iter().cloned().collect();
    let actual_set = executed_set.union(&actual_ignored_set).cloned().collect();
    let missing = expected_set
        .difference(&actual_set)
        .cloned()
        .collect::<Vec<_>>();

    if expected_set != actual_set
        || ignored_set != actual_ignored_set
        || !executed_set.is_disjoint(&actual_ignored_set)
    {
        bail!(
            "test census does not reconcile: missing {}",
            missing.join(", ")
        );
    }
    Ok(missing)
}

impl CoverageStatus {
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("serialize status")
        )
    }

    pub fn from_json(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }

    /// Parse and validate producer evidence regardless of its terminal category.
    pub fn from_validated_json(s: &str) -> Result<Self> {
        let status = Self::from_json(s)?;
        status.validate()?;
        Ok(status)
    }

    /// Parse only a complete producer result suitable for coverage policy.
    pub fn from_completed_json(s: &str) -> Result<Self> {
        let status = Self::from_validated_json(s)?;
        if status.category != StatusCategory::TestsOk {
            bail!("coverage status is not tests-ok");
        }
        Ok(status)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != COVERAGE_STATUS_VERSION {
            bail!("unsupported coverage status version {}", self.version);
        }
        let observed: BTreeSet<_> = self.stages.iter().map(|result| &result.stage).collect();
        if observed.len() != RequiredStage::ALL.len()
            || self.stages.len() != RequiredStage::ALL.len()
            || RequiredStage::ALL
                .iter()
                .any(|stage| !observed.contains(stage))
        {
            bail!("required stages must appear exactly once");
        }
        let stages_ok = self.stages.iter().all(|result| result.outcome.is_success());
        let population_ok = self.population.expected > 0
            && self.population.executed > 0
            && self.population.executed + self.population.ignored == self.population.expected;
        let stage_outcome = |stage| {
            self.stages
                .iter()
                .find(|result| result.stage == stage)
                .expect("required stage")
                .outcome
                .is_success()
        };
        match self.category {
            StatusCategory::TestsOk
                if stages_ok
                    && population_ok
                    && self.failed_tests.is_empty()
                    && self.missing_tests.is_empty()
                    && self.infra_detail.is_none() =>
            {
                Ok(())
            }
            StatusCategory::TestFailure
                if population_ok
                    && !self.failed_tests.is_empty()
                    && self.missing_tests.is_empty()
                    && self.infra_detail.is_none()
                    && matches!(
                        self.stages
                            .iter()
                            .find(|result| result.stage == RequiredStage::InstrumentedTestRun)
                            .expect("required stage")
                            .outcome,
                        ProcessOutcome::ExitCode { exit_code } if exit_code != 0
                    )
                    && RequiredStage::ALL
                        .into_iter()
                        .filter(|stage| *stage != RequiredStage::InstrumentedTestRun)
                        .all(stage_outcome) =>
            {
                Ok(())
            }
            StatusCategory::Infra
                if self
                    .infra_detail
                    .as_deref()
                    .is_some_and(|detail| !detail.is_empty())
                    && self.stages.iter().any(|result| result.outcome.is_failure()) =>
            {
                Ok(())
            }
            _ => bail!("coverage status fields contradict category"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_tests_ok_status() -> Value {
        serde_json::json!({
            "version": 1,
            "category": "tests-ok",
            "stages": [
                {"stage": "workspace-resolution", "outcome": "success"},
                {"stage": "profile-cleanup", "outcome": "success"},
                {"stage": "test-census", "outcome": "success"},
                {"stage": "instrumented-test-run", "outcome": "success"},
                {"stage": "population-reconciliation", "outcome": "success"},
                {"stage": "text-report", "outcome": "success"},
                {"stage": "lcov-report", "outcome": "success"},
                {"stage": "crap-report", "outcome": "success"}
            ],
            "population": {
                "expected": 2,
                "executed": 1,
                "ignored": 1
            },
            "failed_tests": [],
            "missing_tests": [],
            "infra_detail": null
        })
    }

    fn assert_invalid(status: Value) {
        let status = CoverageStatus::from_json(&status.to_string()).expect("status fixture parses");
        assert!(
            status.validate().is_err(),
            "{status:?} unexpectedly validated"
        );
    }

    #[test]
    fn accepts_a_complete_versioned_tests_ok_status() {
        let status = CoverageStatus::from_json(&complete_tests_ok_status().to_string())
            .expect("status fixture parses");
        assert!(status.validate().is_ok());
        assert_eq!(status.category, StatusCategory::TestsOk);
    }

    #[test]
    fn validates_only_the_known_status_version() {
        let mut status = complete_tests_ok_status();
        status["version"] = serde_json::json!(2);
        assert_invalid(status);
    }

    #[test]
    fn requires_each_required_stage_once() {
        let cases = [
            (
                "missing-stage",
                serde_json::json!([
                    {"stage": "workspace-resolution", "outcome": "success"},
                    {"stage": "profile-cleanup", "outcome": "success"},
                    {"stage": "test-census", "outcome": "success"},
                    {"stage": "instrumented-test-run", "outcome": "success"},
                    {"stage": "population-reconciliation", "outcome": "success"},
                    {"stage": "text-report", "outcome": "success"},
                    {"stage": "lcov-report", "outcome": "success"}
                ]),
            ),
            (
                "duplicate-stage",
                serde_json::json!([
                    {"stage": "workspace-resolution", "outcome": "success"},
                    {"stage": "workspace-resolution", "outcome": "success"},
                    {"stage": "profile-cleanup", "outcome": "success"},
                    {"stage": "test-census", "outcome": "success"},
                    {"stage": "instrumented-test-run", "outcome": "success"},
                    {"stage": "population-reconciliation", "outcome": "success"},
                    {"stage": "text-report", "outcome": "success"},
                    {"stage": "lcov-report", "outcome": "success"},
                    {"stage": "crap-report", "outcome": "success"}
                ]),
            ),
        ];

        for (name, stages) in cases {
            let mut status = complete_tests_ok_status();
            status["stages"] = stages;
            let status =
                CoverageStatus::from_json(&status.to_string()).expect("status fixture parses");
            assert!(status.validate().is_err(), "{name} unexpectedly validated");
        }
    }

    #[test]
    fn rejects_tests_ok_contradictions_and_incomplete_populations() {
        let cases = [
            (
                "failed-test",
                serde_json::json!({"failed_tests": ["web::rejects_unauthenticated"]}),
            ),
            (
                "failed-stage",
                serde_json::json!({"stages": [{
                    "stage": "workspace-resolution",
                    "outcome": {"exit-code": {"exit_code": 101}}
                }]}),
            ),
            (
                "empty-population",
                serde_json::json!({"population": {"expected": 0, "executed": 0, "ignored": 0}}),
            ),
            (
                "missing-execution",
                serde_json::json!({"population": {"expected": 2, "executed": 0, "ignored": 1}}),
            ),
            (
                "unreconciled-population",
                serde_json::json!({"population": {"expected": 3, "executed": 1, "ignored": 1}}),
            ),
        ];

        for (_name, patch) in cases {
            let mut status = complete_tests_ok_status();
            for (field, value) in patch.as_object().expect("object patch") {
                status[field] = value.clone();
            }
            assert_invalid(status);
        }
    }

    #[test]
    fn preserves_structured_test_failure_as_distinct_from_infrastructure() {
        let mut status = complete_tests_ok_status();
        status["category"] = serde_json::json!("test-failure");
        status["failed_tests"] = serde_json::json!(["web::rejects_unauthenticated"]);
        let stage_results = status["stages"].as_array_mut().expect("stage array");
        let test_run = stage_results
            .iter_mut()
            .find(|result| result["stage"] == "instrumented-test-run")
            .expect("instrumented test stage");
        test_run["outcome"] = serde_json::json!({"exit-code": {"exit_code": 1}});

        let status = CoverageStatus::from_json(&status.to_string()).expect("status fixture parses");
        assert!(status.validate().is_ok());
        assert_eq!(status.category, StatusCategory::TestFailure);
    }

    #[test]
    fn rejects_test_failure_without_a_nonzero_test_process_exit() {
        for outcome in [
            serde_json::json!({"exit-code": {"exit_code": 0}}),
            serde_json::json!("signal"),
            serde_json::json!({"spawn-error": {"spawn_error": "not-found"}}),
            serde_json::json!({"evidence-error": {"evidence_error": "malformed"}}),
            serde_json::json!("not-run"),
        ] {
            let mut status = complete_tests_ok_status();
            status["category"] = serde_json::json!("test-failure");
            status["failed_tests"] = serde_json::json!(["web::rejects_unauthenticated"]);
            let stages = status["stages"].as_array_mut().expect("stage array");
            stages
                .iter_mut()
                .find(|result| result["stage"] == "instrumented-test-run")
                .expect("test run")["outcome"] = outcome;
            assert_invalid(status);
        }
    }

    #[test]
    fn completed_status_api_requires_valid_tests_ok_evidence() {
        let complete = complete_tests_ok_status().to_string();
        assert!(CoverageStatus::from_completed_json(&complete).is_ok());

        let mut red = complete_tests_ok_status();
        red["category"] = serde_json::json!("infra");
        red["infra_detail"] = serde_json::json!("producer failed");
        red["stages"][0]["outcome"] =
            serde_json::json!({"evidence-error": {"evidence_error": "malformed"}});
        assert!(CoverageStatus::from_validated_json(&red.to_string()).is_ok());
        assert!(CoverageStatus::from_completed_json(&red.to_string()).is_err());
    }

    #[test]
    fn rejects_test_failure_when_a_non_test_stage_is_incomplete() {
        let mut status = complete_tests_ok_status();
        status["category"] = serde_json::json!("test-failure");
        status["failed_tests"] = serde_json::json!(["web::rejects_unauthenticated"]);
        let stages = status["stages"].as_array_mut().expect("stage array");
        stages
            .iter_mut()
            .find(|result| result["stage"] == "instrumented-test-run")
            .expect("test run")["outcome"] = serde_json::json!({"exit-code": {"exit_code": 1}});
        stages
            .iter_mut()
            .find(|result| result["stage"] == "text-report")
            .expect("text report")["outcome"] = serde_json::json!("not-run");
        assert_invalid(status);
    }

    #[test]
    fn rejects_category_field_contradictions() {
        let cases = [
            (
                "test-failure-without-failed-test",
                serde_json::json!({
                    "category": "test-failure",
                    "failed_tests": []
                }),
            ),
            (
                "test-failure-with-incomplete-population",
                serde_json::json!({
                    "category": "test-failure",
                    "failed_tests": ["web::rejects_unauthenticated"],
                    "population": {"expected": 0, "executed": 0, "ignored": 0}
                }),
            ),
            (
                "test-failure-with-missing-tests",
                serde_json::json!({
                    "category": "test-failure",
                    "failed_tests": ["web::rejects_unauthenticated"],
                    "missing_tests": ["web::missing"]
                }),
            ),
            (
                "test-failure-with-infrastructure-detail",
                serde_json::json!({
                    "category": "test-failure",
                    "failed_tests": ["web::rejects_unauthenticated"],
                    "infra_detail": "producer failed"
                }),
            ),
            (
                "infra-without-detail",
                serde_json::json!({
                    "category": "infra",
                    "infra_detail": null
                }),
            ),
            (
                "infra-without-a-failed-stage",
                serde_json::json!({
                    "category": "infra",
                    "infra_detail": "coverage producer failed"
                }),
            ),
        ];

        for (name, patch) in cases {
            let mut status = complete_tests_ok_status();
            for (field, value) in patch.as_object().expect("object patch") {
                status[field] = value.clone();
            }
            let status =
                CoverageStatus::from_json(&status.to_string()).expect("status fixture parses");
            assert!(status.validate().is_err(), "{name} unexpectedly validated");
        }
    }

    #[test]
    fn accepts_a_red_status_for_each_checked_required_stage_outcome() {
        let stages = [
            "workspace-resolution",
            "profile-cleanup",
            "test-census",
            "instrumented-test-run",
            "population-reconciliation",
            "text-report",
            "lcov-report",
            "crap-report",
        ];

        for stage in stages {
            for outcome in [
                serde_json::json!({"exit-code": {"exit_code": 101}}),
                serde_json::json!("signal"),
                serde_json::json!({"spawn-error": {"spawn_error": "not-found"}}),
                serde_json::json!({"evidence-error": {"evidence_error": "malformed"}}),
            ] {
                let mut status = complete_tests_ok_status();
                status["category"] = serde_json::json!("infra");
                status["infra_detail"] = serde_json::json!(format!("{stage} failed"));
                let stage_results = status["stages"].as_array_mut().expect("stage array");
                let stage_result = stage_results
                    .iter_mut()
                    .find(|result| result["stage"] == stage)
                    .expect("required stage");
                stage_result["outcome"] = outcome;

                let status =
                    CoverageStatus::from_json(&status.to_string()).expect("status fixture parses");
                assert!(
                    status.validate().is_ok(),
                    "{stage} failure was not a valid red status"
                );
                assert_ne!(status.category, StatusCategory::TestsOk);
            }
        }
    }

    #[test]
    fn roundtrips_through_json() {
        let s = CoverageStatus {
            version: COVERAGE_STATUS_VERSION,
            category: StatusCategory::TestFailure,
            stages: RequiredStage::ALL
                .into_iter()
                .map(|stage| StageResult {
                    stage,
                    outcome: if stage == RequiredStage::InstrumentedTestRun {
                        ProcessOutcome::ExitCode { exit_code: 1 }
                    } else {
                        ProcessOutcome::success()
                    },
                })
                .collect(),
            population: Population {
                expected: 1,
                executed: 1,
                ignored: 0,
            },
            failed_tests: vec!["web_posts::case_3".into()],
            missing_tests: vec![],
            infra_detail: None,
        };
        let back = CoverageStatus::from_json(&s.to_json()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn category_serializes_kebab_case() {
        let s = CoverageStatus {
            version: COVERAGE_STATUS_VERSION,
            category: StatusCategory::Infra,
            stages: RequiredStage::ALL
                .into_iter()
                .map(|stage| StageResult {
                    stage,
                    outcome: if stage == RequiredStage::WorkspaceResolution {
                        ProcessOutcome::EvidenceError {
                            evidence_error: "metadata failed".into(),
                        }
                    } else {
                        ProcessOutcome::success()
                    },
                })
                .collect(),
            population: Population {
                expected: 1,
                executed: 1,
                ignored: 0,
            },
            failed_tests: vec![],
            missing_tests: vec![],
            infra_detail: Some("ENOSPC".into()),
        };
        assert!(s.to_json().contains("\"infra\""));
    }

    #[test]
    fn parses_stable_nextest_and_junit_identities() {
        let expected = parse_nextest_census(
            r#"{
                "test-count": 2,
                "rust-suites": {
                    "server": {
                        "binary-id": "server",
                        "testcases": {
                            "passes": {"ignored": false, "filter-match": {"status": "matches"}},
                            "ignored": {
                                "ignored": true,
                                "filter-match": {"status": "mismatch", "reason": "ignored"}
                            }
                        }
                    }
                }
            }"#,
        )
        .expect("stable nextest list");
        let actual = parse_junit_census(
            r#"<testsuites>
                <testcase classname="server" name="passes"/>
                <testcase classname="server" name="ignored"><skipped/></testcase>
            </testsuites>"#,
        )
        .expect("JUnit results");
        assert_eq!(
            expected.expected,
            vec!["server::ignored".to_owned(), "server::passes".to_owned()]
        );
        assert_eq!(expected.ignored, vec!["server::ignored".to_owned()]);
        assert_eq!(actual.executed, vec!["server::passes".to_owned()]);
        assert_eq!(actual.ignored, vec!["server::ignored".to_owned()]);
        assert_eq!(
            reconcile_test_census(&expected, &actual).expect("exact census"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn rejects_nonexact_or_malformed_censuses() {
        for testcase in [
            r#"{"ignored": false, "filter-match": {"status": "mismatch", "reason": "ignored"}}"#,
            r#"{"ignored": true, "filter-match": {"status": "mismatch", "reason": "default-filter"}}"#,
            r#"{"ignored": true, "filter-match": {"status": "matches"}}"#,
            r#"{"ignored": false, "filter-match": {"status": "mismatch"}}"#,
        ] {
            assert!(
                parse_nextest_census(&format!(
                    r#"{{"test-count": 1, "rust-suites": {{"suite": {{"binary-id": "bin", "testcases": {{"test": {testcase}}}}}}}}}"#
                ))
                .is_err()
            );
        }
        assert!(parse_junit_census(r#"<testcase name="missing-classname"/>"#).is_err());
        assert!(
            parse_junit_census(
                r#"<testcase classname="bin" name="test"/><testcase classname="bin" name="test"/>"#,
            )
            .is_err()
        );
        assert!(
            parse_junit_census(
                r#"<testcase classname="bin" name="contradictory"><skipped/><failure/></testcase>"#
            )
            .is_err()
        );

        let expected = parse_nextest_census(
            r#"{"test-count": 1, "rust-suites": {"suite": {"binary-id": "bin", "testcases": {"test": {"ignored": false, "filter-match": {"status": "matches"}}}}}}"#,
        )
        .expect("expected census");
        let actual = parse_junit_census(r#"<testcase classname="bin" name="other"/>"#)
            .expect("actual census");
        assert!(reconcile_test_census(&expected, &actual).is_err());
    }
}

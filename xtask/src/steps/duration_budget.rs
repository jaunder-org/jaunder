//! Strict host-side reconciliation of a Playwright JSON report with the
//! Playwright-resolved duration-budget manifest.
//!
//! The manifest owns the selected population and each attempt's final effective
//! timeout. The report owns measured durations. Neither input is trustworthy on
//! its own: this module accepts a combo only when their identities and retry
//! streams agree exactly, then rejects the greatest observed utilization at or
//! above the pressure threshold.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::StepResult;

const SCHEMA_VERSION: u32 = 1;
const PRESSURE_THRESHOLD: f64 = 0.80;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    test_id: String,
    project_id: String,
    project_name: String,
    title: String,
    file: String,
    line: u64,
}

impl Identity {
    fn describe(&self) -> String {
        format!(
            "test_id={} project_id={} project_name={} {}:{} title={}",
            self.test_id, self.project_id, self.project_name, self.file, self.line, self.title
        )
    }

    fn validate(&self, source: &str) -> Result<(), String> {
        for (name, value) in [
            ("test_id", &self.test_id),
            ("project_id", &self.project_id),
            ("project_name", &self.project_name),
            ("title", &self.title),
            ("file", &self.file),
        ] {
            if value.is_empty() {
                return Err(format!("{source} has an empty {name}"));
            }
        }
        if self.line == 0 {
            return Err(format!("{source} has line 0"));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    complete: bool,
    tests: Vec<ManifestTest>,
}

#[derive(Deserialize)]
struct ManifestTest {
    test_id: String,
    project_id: String,
    project_name: String,
    title: String,
    file: String,
    line: u64,
    attempts: Vec<ManifestAttempt>,
}

impl ManifestTest {
    fn identity(&self) -> Identity {
        Identity {
            test_id: self.test_id.clone(),
            project_id: self.project_id.clone(),
            project_name: self.project_name.clone(),
            title: self.title.clone(),
            file: self.file.clone(),
            line: self.line,
        }
    }
}

#[derive(Deserialize)]
struct ManifestAttempt {
    retry: u32,
    effective_timeout_ms: f64,
}

#[derive(Deserialize)]
struct Report {
    suites: Vec<ReportSuite>,
}

#[derive(Deserialize)]
struct ReportSuite {
    #[serde(default)]
    suites: Vec<ReportSuite>,
    #[serde(default)]
    specs: Vec<ReportSpec>,
}

#[derive(Deserialize)]
struct ReportSpec {
    id: String,
    title: String,
    file: String,
    line: u64,
    tests: Vec<ReportTest>,
}

#[derive(Deserialize)]
struct ReportTest {
    #[serde(rename = "projectId")]
    project_id: String,
    #[serde(rename = "projectName")]
    project_name: String,
    results: Vec<ReportAttempt>,
}

#[derive(Deserialize)]
struct ReportAttempt {
    retry: u32,
    duration: f64,
}

#[derive(Clone, Copy)]
struct Offender<'a> {
    identity: &'a Identity,
    retry: u32,
    duration_ms: f64,
    effective_timeout_ms: f64,
    utilization: f64,
}

/// Validate a lifted combo's two authoritative duration inputs.
///
/// This runs only after a successful VM check and after the diagnostic lift, so
/// an input failure is loud without replacing the primary failure diagnostics.
pub(crate) fn validate_lifted_combo(backend: &str, browser: &str) -> StepResult {
    let diagnostics = Path::new(".xtask/diagnostics").join(format!("e2e-{backend}-{browser}"));
    let report = diagnostics.join(format!("playwright-report-{backend}.json"));
    let manifest = diagnostics.join(format!("duration-budget-manifest-{backend}.json"));

    match validate_files(&report, &manifest) {
        Ok(detail) => StepResult::ok("e2e-duration-budget").detail(detail),
        Err(error) => StepResult::fail("e2e-duration-budget").detail(format!(
            "e2e {backend}/{browser} duration-budget input is unavailable or inconsistent: {error}"
        )),
    }
}

fn validate_files(report_path: &Path, manifest_path: &Path) -> Result<String, String> {
    let report = std::fs::read_to_string(report_path)
        .map_err(|error| format!("reading report {}: {error}", report_path.display()))?;
    let manifest = std::fs::read_to_string(manifest_path)
        .map_err(|error| format!("reading manifest {}: {error}", manifest_path.display()))?;
    validate_json(&report, &manifest)
}

fn validate_json(report_json: &str, manifest_json: &str) -> Result<String, String> {
    let report: Report = serde_json::from_str(report_json)
        .map_err(|error| format!("parsing Playwright report: {error}"))?;
    let manifest: Manifest = serde_json::from_str(manifest_json)
        .map_err(|error| format!("parsing duration-budget manifest: {error}"))?;

    if manifest.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported manifest schema_version {}; expected {SCHEMA_VERSION}",
            manifest.schema_version
        ));
    }
    if !manifest.complete {
        return Err("manifest is not complete".into());
    }

    let expected = manifest_attempts(manifest)?;
    let observed = report_attempts(report)?;
    reconcile(&expected, &observed)
}

fn manifest_attempts(manifest: Manifest) -> Result<BTreeMap<Identity, BTreeMap<u32, f64>>, String> {
    if manifest.tests.is_empty() {
        return Err("manifest selects no tests".into());
    }

    let mut expected = BTreeMap::new();
    for test in manifest.tests {
        let identity = test.identity();
        identity.validate("manifest test")?;
        if test.attempts.is_empty() {
            return Err(format!(
                "manifest test has no attempts: {}",
                identity.describe()
            ));
        }
        let mut attempts = BTreeMap::new();
        for attempt in test.attempts {
            if !attempt.effective_timeout_ms.is_finite() || attempt.effective_timeout_ms <= 0.0 {
                return Err(format!(
                    "manifest attempt retry {} has a non-positive or non-finite effective timeout: {}",
                    attempt.retry,
                    identity.describe()
                ));
            }
            if attempts
                .insert(attempt.retry, attempt.effective_timeout_ms)
                .is_some()
            {
                return Err(format!(
                    "manifest has duplicate retry {}: {}",
                    attempt.retry,
                    identity.describe()
                ));
            }
        }
        require_contiguous_retries(attempts.keys().copied(), "manifest", &identity)?;
        if expected.insert(identity.clone(), attempts).is_some() {
            return Err(format!(
                "manifest has a duplicate selected test: {}",
                identity.describe()
            ));
        }
    }
    Ok(expected)
}

fn report_attempts(report: Report) -> Result<BTreeMap<Identity, BTreeMap<u32, f64>>, String> {
    let mut specs = Vec::new();
    for suite in report.suites {
        collect_specs(suite, &mut specs);
    }
    if specs.is_empty() {
        return Err("report contains no selected tests".into());
    }

    let mut observed = BTreeMap::new();
    for spec in specs {
        if spec.tests.is_empty() {
            return Err(format!("report spec {} has no project results", spec.id));
        }
        for test in spec.tests {
            let identity = Identity {
                test_id: spec.id.clone(),
                project_id: test.project_id,
                project_name: test.project_name,
                title: spec.title.clone(),
                file: spec.file.clone(),
                line: spec.line,
            };
            identity.validate("report test")?;
            if test.results.is_empty() {
                return Err(format!(
                    "report test has no results: {}",
                    identity.describe()
                ));
            }
            let mut attempts = BTreeMap::new();
            for attempt in test.results {
                if !attempt.duration.is_finite() || attempt.duration < 0.0 {
                    return Err(format!(
                        "report retry {} has a non-finite or negative duration: {}",
                        attempt.retry,
                        identity.describe()
                    ));
                }
                if attempts.insert(attempt.retry, attempt.duration).is_some() {
                    return Err(format!(
                        "report has duplicate retry {}: {}",
                        attempt.retry,
                        identity.describe()
                    ));
                }
            }
            require_contiguous_retries(attempts.keys().copied(), "report", &identity)?;
            if observed.insert(identity.clone(), attempts).is_some() {
                return Err(format!(
                    "report has a duplicate selected test: {}",
                    identity.describe()
                ));
            }
        }
    }
    Ok(observed)
}

fn collect_specs(suite: ReportSuite, specs: &mut Vec<ReportSpec>) {
    specs.extend(suite.specs);
    for child in suite.suites {
        collect_specs(child, specs);
    }
}

fn require_contiguous_retries(
    retries: impl Iterator<Item = u32>,
    source: &str,
    identity: &Identity,
) -> Result<(), String> {
    for (expected, actual) in retries.enumerate() {
        if actual != expected as u32 {
            return Err(format!(
                "{source} retries must start at zero and be contiguous; expected {expected}, found {actual}: {}",
                identity.describe()
            ));
        }
    }
    Ok(())
}

fn reconcile(
    expected: &BTreeMap<Identity, BTreeMap<u32, f64>>,
    observed: &BTreeMap<Identity, BTreeMap<u32, f64>>,
) -> Result<String, String> {
    let expected_ids: BTreeSet<_> = expected.keys().collect();
    let observed_ids: BTreeSet<_> = observed.keys().collect();
    if expected_ids != observed_ids {
        let missing = expected_ids.difference(&observed_ids).next();
        let unexpected = observed_ids.difference(&expected_ids).next();
        return Err(match (missing, unexpected) {
            (Some(missing), Some(unexpected)) => format!(
                "selected-test identity mismatch; missing {} and unexpected {}",
                missing.describe(),
                unexpected.describe()
            ),
            (Some(missing), None) => {
                format!("report is missing selected test: {}", missing.describe())
            }
            (None, Some(unexpected)) => {
                format!(
                    "report has unexpected selected test: {}",
                    unexpected.describe()
                )
            }
            (None, None) => unreachable!("equal identity sets returned mismatch"),
        });
    }

    let mut maximum: Option<Offender<'_>> = None;
    for (identity, timeouts) in expected {
        let durations = &observed[identity];
        let expected_retries: BTreeSet<_> = timeouts.keys().copied().collect();
        let observed_retries: BTreeSet<_> = durations.keys().copied().collect();
        if expected_retries != observed_retries {
            return Err(format!(
                "retry reconciliation mismatch: {}",
                identity.describe()
            ));
        }
        for (&retry, &effective_timeout_ms) in timeouts {
            let duration_ms = durations[&retry];
            let utilization = duration_ms / effective_timeout_ms;
            let candidate = Offender {
                identity,
                retry,
                duration_ms,
                effective_timeout_ms,
                utilization,
            };
            if maximum.is_none_or(|prior| candidate.utilization > prior.utilization) {
                maximum = Some(candidate);
            }
        }
    }

    let maximum = maximum.expect("nonempty reconciled manifest has attempts");
    if maximum.utilization >= PRESSURE_THRESHOLD {
        return Err(format!(
            "duration pressure {:.2}% (threshold 80.00%): {}; retry {}; duration {:.3} ms; effective timeout {:.3} ms",
            maximum.utilization * 100.0,
            maximum.identity.describe(),
            maximum.retry,
            maximum.duration_ms,
            maximum.effective_timeout_ms,
        ));
    }
    Ok(format!(
        "maximum duration pressure {:.2}%: {}; retry {}; duration {:.3} ms / effective timeout {:.3} ms",
        maximum.utilization * 100.0,
        maximum.identity.describe(),
        maximum.retry,
        maximum.duration_ms,
        maximum.effective_timeout_ms,
    ))
}

#[cfg(test)]
mod tests {
    use super::{validate_files, validate_json};

    const REPORT: &str = r#"{
      "suites": [{"title":"tests", "file":"", "line":0, "specs":[{
        "id":"id-1", "title":"works", "file":"tests/works.spec.ts", "line":7,
        "tests":[{"projectId":"chromium", "projectName":"chromium", "results":[{"retry":0,"duration":100}]}]
      }]}]
    }"#;
    const MANIFEST: &str = r#"{
      "schema_version":1,"complete":true,"tests":[{
        "test_id":"id-1", "project_id":"chromium", "project_name":"chromium", "title":"works", "file":"tests/works.spec.ts", "line":7,
        "attempts":[{"retry":0,"effective_timeout_ms":1000}]
      }]
    }"#;

    fn validate(report: &str, manifest: &str) -> Result<String, String> {
        validate_json(report, manifest)
    }

    #[test]
    fn accepts_attempt_below_pressure_threshold() {
        assert!(validate(REPORT, MANIFEST).is_ok());
    }

    #[test]
    fn rejects_attempt_at_pressure_threshold() {
        let report = REPORT.replace("\"duration\":100", "\"duration\":800");
        let error = validate(&report, MANIFEST).unwrap_err();
        assert!(error.contains("80.00%"), "{error}");
    }

    #[test]
    fn rejects_attempt_above_pressure_threshold() {
        let report = REPORT.replace("\"duration\":100", "\"duration\":801");
        assert!(validate(&report, MANIFEST).unwrap_err().contains("80.10%"));
    }

    #[test]
    fn accepts_all_effective_timeout_modes() {
        for timeout in [1_000, 3_000, 12_345] {
            let manifest = MANIFEST.replace("1000", &timeout.to_string());
            assert!(validate(REPORT, &manifest).is_ok(), "timeout {timeout}");
        }
    }

    #[test]
    fn rejects_slow_first_retry_even_when_later_retry_passes() {
        let report = REPORT.replace(
            "{\"retry\":0,\"duration\":100}",
            "{\"retry\":0,\"duration\":900},{\"retry\":1,\"duration\":100}",
        );
        let manifest = MANIFEST.replace(
            "[{\"retry\":0,\"effective_timeout_ms\":1000}]",
            "[{\"retry\":0,\"effective_timeout_ms\":1000},{\"retry\":1,\"effective_timeout_ms\":1000}]",
        );
        assert!(
            validate(&report, &manifest)
                .unwrap_err()
                .contains("retry 0")
        );
    }

    #[test]
    fn rejects_empty_or_malformed_report() {
        assert!(
            validate("{\"suites\":[]}", MANIFEST)
                .unwrap_err()
                .contains("no selected tests")
        );
        assert!(
            validate("not json", MANIFEST)
                .unwrap_err()
                .contains("parsing Playwright report")
        );
    }

    #[test]
    fn rejects_incomplete_or_missing_manifest_content() {
        assert!(
            validate(
                REPORT,
                "{\"schema_version\":1,\"complete\":false,\"tests\":[]}"
            )
            .unwrap_err()
            .contains("not complete")
        );
        assert!(
            validate(REPORT, "{}")
                .unwrap_err()
                .contains("parsing duration-budget manifest")
        );
    }

    #[test]
    fn rejects_identity_mismatch() {
        let report = REPORT.replace("\"id-1\"", "\"other-id\"");
        assert!(
            validate(&report, MANIFEST)
                .unwrap_err()
                .contains("identity mismatch")
        );
    }

    #[test]
    fn rejects_pruned_report_missing_a_selected_test() {
        let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        let mut omitted = manifest["tests"][0].clone();
        omitted["test_id"] = serde_json::Value::String("id-2".into());
        manifest["tests"].as_array_mut().unwrap().push(omitted);

        assert!(
            validate(REPORT, &serde_json::to_string(&manifest).unwrap())
                .unwrap_err()
                .contains("report is missing selected test")
        );
    }

    #[test]
    fn rejects_non_contiguous_report_retries() {
        let report = REPORT.replace("\"retry\":0", "\"retry\":1");
        assert!(
            validate(&report, MANIFEST)
                .unwrap_err()
                .contains("must start at zero and be contiguous")
        );
    }

    #[test]
    fn rejects_missing_retry_budget() {
        let report = REPORT.replace(
            "{\"retry\":0,\"duration\":100}",
            "{\"retry\":0,\"duration\":100},{\"retry\":1,\"duration\":100}",
        );
        assert!(
            validate(&report, MANIFEST)
                .unwrap_err()
                .contains("retry reconciliation mismatch")
        );
    }

    #[test]
    fn rejects_non_contiguous_manifest_retries() {
        let manifest = MANIFEST.replace("\"retry\":0", "\"retry\":1");
        assert!(
            validate(REPORT, &manifest)
                .unwrap_err()
                .contains("must start at zero and be contiguous")
        );
    }

    #[test]
    fn rejects_missing_manifest_file() {
        let temporary = tempfile::tempdir().unwrap();
        let report = temporary.path().join("report.json");
        let manifest = temporary.path().join("missing-manifest.json");
        std::fs::write(&report, REPORT).unwrap();

        assert!(
            validate_files(&report, &manifest)
                .unwrap_err()
                .contains("reading manifest")
        );
    }
}

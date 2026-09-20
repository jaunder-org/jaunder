//! Fail-closed aggregate validation for every distributed E2E lane.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::e2e_lanes::Lane;
use crate::result::{PhaseName, PhaseOutcome, PhaseRecord};

#[derive(Clone)]
pub struct LaneEvidence {
    pub identity: String,
    pub census: PathBuf,
    pub report: PathBuf,
    pub lane_manifest: PathBuf,
    pub duration_manifest: PathBuf,
    pub capture: PathBuf,
    pub phase: PathBuf,
}

#[derive(Deserialize)]
struct PhaseSidecar {
    schema_version: u8,
    backend: String,
    browser: String,
    lane: String,
    phases: Vec<PhaseRecord>,
}

fn read(path: &Path, kind: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("reading {kind} {}: {error}", path.display()))
}

fn validate_phase_sidecar(
    sidecar: &PhaseSidecar,
    backend: &str,
    browser: &str,
    lane: &str,
) -> Result<(), String> {
    if sidecar.schema_version != 1
        || sidecar.backend != backend
        || sidecar.browser != browser
        || sidecar.lane != lane
    {
        return Err("identity mismatch".into());
    }
    for required in [
        PhaseName::GateExecution,
        PhaseName::ResultLift,
        PhaseName::PostGateChecks,
    ] {
        let matches = sidecar
            .phases
            .iter()
            .filter(|phase| phase.name == required)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!("requires exactly one {required:?} phase"));
        }
        let phase = matches[0];
        if phase.outcome != PhaseOutcome::Success
            || phase.duration_ms.is_none()
            || phase.detail.is_empty()
        {
            return Err(format!("{required:?} phase is incomplete or failed"));
        }
    }
    if sidecar
        .phases
        .iter()
        .any(|phase| phase.outcome == PhaseOutcome::Failed)
    {
        return Err("records a failed phase".into());
    }
    Ok(())
}

/// Reconcile all candidate lanes. Ownership runs first; every remaining consumer
/// then validates its lane-qualified input independently, retaining no merged or
/// basename-selected evidence.
#[derive(Deserialize)]
struct PopulationCensus {
    schema_version: u8,
    complete: bool,
    tests: Vec<PopulationTest>,
}

#[derive(Deserialize)]
struct PopulationTest {
    file: String,
    line: u64,
    column: u64,
    title_path: Vec<String>,
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceIdentity {
    file: String,
    line: u64,
    column: u64,
    title_path: Vec<String>,
}

fn source_population(raw: &str, kind: &str) -> Result<BTreeSet<SourceIdentity>, String> {
    let census: PopulationCensus =
        serde_json::from_str(raw).map_err(|error| format!("parsing {kind}: {error}"))?;
    if census.schema_version != 1 || !census.complete || census.tests.is_empty() {
        return Err(format!("{kind} is incomplete or empty"));
    }
    let count = census.tests.len();
    let tests = census
        .tests
        .into_iter()
        .map(|test| {
            if test.file.is_empty()
                || test.line == 0
                || test.column == 0
                || test.title_path.is_empty()
            {
                Err(format!("{kind} has malformed stable test identity"))
            } else {
                Ok(SourceIdentity {
                    file: test.file,
                    line: test.line,
                    column: test.column,
                    title_path: test.title_path,
                })
            }
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if tests.len() != count {
        return Err(format!("{kind} has duplicate stable test identity"));
    }
    Ok(tests)
}

/// Compare a reconciled split run with an unsplit control that actually ran.
/// The census source identity is stable across project topology while each side's
/// report remains independently validated by its owning reconciliation path.
pub fn compare_control_population(
    backend: &str,
    split: &[LaneEvidence],
    lanes: &[Lane],
    control_topology: &str,
    control_census: &Path,
    control_report: &Path,
    control_manifest: &Path,
) -> Result<String, String> {
    let mut candidate_censuses = split.iter().map(|lane| {
        source_population(&read(&lane.census, "candidate census")?, "candidate census")
    });
    let Some(candidate_population) = candidate_censuses.next() else {
        return Err("no candidate census evidence".into());
    };
    let candidate_population = candidate_population?;
    for population in candidate_censuses {
        if population? != candidate_population {
            return Err("candidate censuses disagree on stable test population".into());
        }
    }
    let control_lane = lanes
        .iter()
        .find(|lane| {
            lane.backend == backend
                && lane.browser == "firefox"
                && !lane.enabled
                && lane.partition == "unsplit"
        })
        .ok_or_else(|| "missing Firefox unsplit measurement control lane".to_owned())?;
    let control_census_raw = read(control_census, "control census")?;
    let control_report_raw = read(control_report, "control Playwright report")?;
    let control_manifest_raw = read(control_manifest, "control lane manifest")?;
    crate::e2e_ownership::validate_unsplit_control(
        backend,
        "firefox",
        control_lane,
        control_topology,
        &control_census_raw,
        &control_report_raw,
        &control_manifest_raw,
    )?;
    let control_population = source_population(&control_census_raw, "control census")?;
    if candidate_population != control_population {
        return Err("split/control stable test populations differ".into());
    }
    Ok(format!(
        "split/control population comparison passed for {} tests",
        candidate_population.len()
    ))
}

pub fn reconcile(
    backend: &str,
    browser: &str,
    lanes: &[Lane],
    evidence: &[LaneEvidence],
) -> Result<String, Vec<String>> {
    let mut errors = Vec::new();
    let supplied = evidence
        .iter()
        .map(|item| item.identity.as_str())
        .collect::<BTreeSet<_>>();
    if supplied.len() != evidence.len() {
        errors.push("duplicate lane evidence".into());
    }
    let ownership = evidence
        .iter()
        .map(|item| {
            Ok((
                item.identity.clone(),
                read(&item.census, "expected census")?,
                read(&item.report, "Playwright report")?,
                read(&item.lane_manifest, "lane manifest")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>();
    match ownership {
        Ok(ownership) => {
            if let Err(error) = crate::e2e_ownership::reconcile(backend, browser, lanes, &ownership)
            {
                errors.push(format!("ownership census: {error}"));
            }
        }
        Err(error) => errors.push(error),
    }

    let expected = lanes
        .iter()
        .filter(|lane| {
            lane.backend == backend
                && lane.browser == browser
                && lane.enabled
                && lane.partition != "unsplit"
        })
        .map(|lane| lane.identity.as_str())
        .collect::<BTreeSet<_>>();
    if supplied != expected {
        errors.push("lane evidence does not exactly match the catalog".into());
    }
    for item in evidence {
        let Some(lane) = lanes.iter().find(|lane| lane.identity == item.identity) else {
            continue;
        };
        if lane.backend != backend
            || lane.browser != browser
            || !lane.enabled
            || lane.partition == "unsplit"
        {
            errors.push(format!("unexpected lane `{}`", item.identity));
            continue;
        }
        if let Err(error) =
            crate::steps::duration_budget::validate_files(&item.report, &item.duration_manifest)
        {
            errors.push(format!("{} duration evidence: {error}", item.identity));
        }
        if let Err(error) =
            crate::steps::boot_decomposition_coverage::validate_files(&item.report, &item.capture)
        {
            errors.push(format!("{} trace evidence: {error}", item.identity));
        }
        match read(&item.phase, "phase sidecar").and_then(|raw| {
            serde_json::from_str::<PhaseSidecar>(&raw)
                .map_err(|error| format!("parsing phase sidecar {}: {error}", item.phase.display()))
        }) {
            Ok(sidecar) => {
                if let Err(error) =
                    validate_phase_sidecar(&sidecar, backend, browser, &item.identity)
                {
                    errors.push(format!("{} phase sidecar: {error}", item.identity));
                }
            }
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(format!(
            "{backend}/{browser}: reconciled {} lane-qualified evidence sets",
            evidence.len()
        ))
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::Value;

    use super::{LaneEvidence, reconcile};
    use crate::e2e_lanes::catalog;
    use crate::traces::run::TRACE_MEMBER;

    const CENSUS: &str = include_str!("testdata/e2e-ownership/expected-census.json");
    const REPORTS: [&str; 3] = [
        include_str!("testdata/e2e-ownership/ordinary-1-report.json"),
        include_str!("testdata/e2e-ownership/ordinary-2-report.json"),
        include_str!("testdata/e2e-ownership/serial-special-report.json"),
    ];
    const LANE_MANIFESTS: [&str; 3] = [
        include_str!("testdata/e2e-ownership/ordinary-1-manifest.json"),
        include_str!("testdata/e2e-ownership/ordinary-2-manifest.json"),
        include_str!("testdata/e2e-ownership/serial-special-manifest.json"),
    ];
    const DURATION_MANIFESTS: [&str; 3] = [
        include_str!("testdata/e2e-evidence/ordinary-1-duration.json"),
        include_str!("testdata/e2e-evidence/ordinary-2-duration.json"),
        include_str!("testdata/e2e-evidence/serial-special-duration.json"),
    ];
    const TRACES: [&str; 3] = [
        include_str!("testdata/e2e-evidence/ordinary-1-trace.jsonl"),
        include_str!("testdata/e2e-evidence/ordinary-2-trace.jsonl"),
        include_str!("testdata/e2e-evidence/serial-special-trace.jsonl"),
    ];
    const PHASES: [&str; 3] = [
        include_str!("testdata/e2e-evidence/ordinary-1-phase.json"),
        include_str!("testdata/e2e-evidence/ordinary-2-phase.json"),
        include_str!("testdata/e2e-evidence/serial-special-phase.json"),
    ];
    const IDENTITIES: [&str; 3] = [
        "sqlite-firefox-ordinary-1-of-2",
        "sqlite-firefox-ordinary-2-of-2",
        "sqlite-firefox-serial-special",
    ];

    fn write_capture(path: &Path, trace: &str) {
        let file = fs::File::create(path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(trace.len().try_into().unwrap());
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, TRACE_MEMBER, trace.as_bytes())
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
    }

    fn fixture(root: &Path) -> Vec<LaneEvidence> {
        IDENTITIES
            .iter()
            .enumerate()
            .map(|(index, identity)| {
                let directory = root.join(identity);
                fs::create_dir_all(&directory).unwrap();
                let census = directory.join("census.json");
                let report = directory.join("report.json");
                let lane_manifest = directory.join("lane-manifest.json");
                let duration_manifest = directory.join("duration-manifest.json");
                let capture = directory.join("capture.tar.gz");
                let phase = directory.join("phase.json");
                fs::write(&census, CENSUS).unwrap();
                fs::write(&report, REPORTS[index]).unwrap();
                fs::write(&lane_manifest, LANE_MANIFESTS[index]).unwrap();
                fs::write(&duration_manifest, DURATION_MANIFESTS[index]).unwrap();
                write_capture(&capture, TRACES[index]);
                fs::write(&phase, PHASES[index]).unwrap();
                LaneEvidence {
                    identity: (*identity).to_owned(),
                    census,
                    report,
                    lane_manifest,
                    duration_manifest,
                    capture,
                    phase,
                }
            })
            .collect()
    }

    fn errors(evidence: &[LaneEvidence]) -> Vec<String> {
        reconcile("sqlite", "firefox", &catalog().unwrap().lanes, evidence).unwrap_err()
    }

    fn source_census(topology: &str) -> Value {
        let mut census: Value = serde_json::from_str(CENSUS).unwrap();
        census["topology"] = Value::String(topology.to_owned());
        for (index, test) in census["tests"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            test["file"] = Value::String(format!("tests/source-{index}.spec.ts"));
            test["line"] = Value::from((index + 1) as u64);
            test["column"] = Value::from(1_u64);
            test["title_path"] = Value::Array(vec![Value::String(format!("source {index}"))]);
        }
        census
    }

    fn control_report() -> Value {
        let mut report = serde_json::json!({ "suites": [] });
        let suites = report["suites"].as_array_mut().unwrap();
        for raw in REPORTS {
            let source: Value = serde_json::from_str(raw).unwrap();
            suites.extend(source["suites"].as_array().unwrap().iter().cloned());
        }
        report
    }

    fn control_manifest() -> Value {
        let mut tests = Vec::new();
        for raw in LANE_MANIFESTS {
            let source: Value = serde_json::from_str(raw).unwrap();
            tests.extend(source["tests"].as_array().unwrap().iter().cloned());
        }
        serde_json::json!({
            "schema_version": 1,
            "complete": true,
            "lane": "sqlite-firefox-unsplit",
            "backend": "sqlite",
            "browser": "firefox",
            "partition": "unsplit",
            "shard_index": null,
            "shard_count": null,
            "tests": tests,
        })
    }

    fn control_comparison_fixture(
        root: &Path,
    ) -> (
        Vec<LaneEvidence>,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let evidence = fixture(root);
        let candidate_census = source_census("sqlite-firefox-experimental");
        for lane in &evidence {
            fs::write(&lane.census, serde_json::to_vec(&candidate_census).unwrap()).unwrap();
        }
        let control = root.join("control");
        fs::create_dir_all(&control).unwrap();
        let census = control.join("census.json");
        let report = control.join("report.json");
        let manifest = control.join("manifest.json");
        fs::write(
            &census,
            serde_json::to_vec(&source_census("sqlite-firefox-workers-2-control")).unwrap(),
        )
        .unwrap();
        fs::write(&report, serde_json::to_vec(&control_report()).unwrap()).unwrap();
        fs::write(&manifest, serde_json::to_vec(&control_manifest()).unwrap()).unwrap();
        (evidence, census, report, manifest)
    }

    fn compare(
        evidence: &[LaneEvidence],
        census: &Path,
        report: &Path,
        manifest: &Path,
    ) -> Result<String, String> {
        super::compare_control_population(
            "sqlite",
            evidence,
            &catalog().unwrap().lanes,
            "sqlite-firefox-workers-2-control",
            census,
            report,
            manifest,
        )
    }

    #[test]
    fn complete_three_lane_fixture_reconciles() {
        let temp = tempfile::tempdir().unwrap();
        let evidence = fixture(temp.path());
        let detail = reconcile("sqlite", "firefox", &catalog().unwrap().lanes, &evidence).unwrap();
        assert!(detail.contains("3 lane-qualified evidence sets"));
    }

    #[test]
    fn missing_duplicate_and_malformed_lane_evidence_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let mut evidence = fixture(temp.path());
        evidence.pop();
        assert!(
            errors(&evidence)
                .iter()
                .any(|error| error.contains("exactly match"))
        );

        let temp = tempfile::tempdir().unwrap();
        let mut evidence = fixture(temp.path());
        evidence.push(evidence[0].clone());
        assert!(
            errors(&evidence)
                .iter()
                .any(|error| error.contains("duplicate lane"))
        );

        let temp = tempfile::tempdir().unwrap();
        let evidence = fixture(temp.path());
        fs::write(&evidence[0].report, "{").unwrap();
        assert!(
            errors(&evidence)
                .iter()
                .any(|error| error.contains("report"))
        );
    }

    #[test]
    fn duplicate_attempt_and_partial_trace_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let evidence = fixture(temp.path());
        let mut duration: Value =
            serde_json::from_str(&fs::read_to_string(&evidence[0].duration_manifest).unwrap())
                .unwrap();
        let attempt = duration["tests"][0]["attempts"][0].clone();
        duration["tests"][0]["attempts"]
            .as_array_mut()
            .unwrap()
            .push(attempt);
        fs::write(
            &evidence[0].duration_manifest,
            serde_json::to_vec(&duration).unwrap(),
        )
        .unwrap();
        assert!(
            errors(&evidence)
                .iter()
                .any(|error| error.contains("duplicate retry"))
        );

        let temp = tempfile::tempdir().unwrap();
        let evidence = fixture(temp.path());
        let dropped = TRACES[0].replace(
            "e2e.navigation_top_dropped\",\"value\":{\"stringValue\":\"0\"",
            "e2e.navigation_top_dropped\",\"value\":{\"stringValue\":\"1\"",
        );
        write_capture(&evidence[0].capture, &dropped);
        assert!(
            errors(&evidence)
                .iter()
                .any(|error| error.contains("dropped"))
        );
    }

    #[test]
    fn control_population_comparison_rejects_inconsistent_or_unexecuted_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let (evidence, census, report, manifest) = control_comparison_fixture(temp.path());
        assert!(compare(&evidence, &census, &report, &manifest).is_ok());

        let temp = tempfile::tempdir().unwrap();
        let (evidence, census, report, manifest) = control_comparison_fixture(temp.path());
        let mut disagree: Value =
            serde_json::from_str(&fs::read_to_string(&evidence[1].census).unwrap()).unwrap();
        disagree["tests"][0]["line"] = Value::from(99_u64);
        fs::write(&evidence[1].census, serde_json::to_vec(&disagree).unwrap()).unwrap();
        assert!(
            compare(&evidence, &census, &report, &manifest)
                .unwrap_err()
                .contains("candidate censuses disagree")
        );

        let temp = tempfile::tempdir().unwrap();
        let (evidence, census, report, manifest) = control_comparison_fixture(temp.path());
        let mut mismatch: Value =
            serde_json::from_str(&fs::read_to_string(&census).unwrap()).unwrap();
        mismatch["tests"][0]["line"] = Value::from(99_u64);
        fs::write(&census, serde_json::to_vec(&mismatch).unwrap()).unwrap();
        assert!(
            compare(&evidence, &census, &report, &manifest)
                .unwrap_err()
                .contains("stable test populations differ")
        );

        let temp = tempfile::tempdir().unwrap();
        let (evidence, census, report, manifest) = control_comparison_fixture(temp.path());
        fs::write(&report, "{").unwrap();
        assert!(
            compare(&evidence, &census, &report, &manifest)
                .unwrap_err()
                .contains("malformed")
        );

        let temp = tempfile::tempdir().unwrap();
        let (evidence, census, report, manifest) = control_comparison_fixture(temp.path());
        fs::write(&report, r#"{"suites":[]}"#).unwrap();
        assert!(
            compare(&evidence, &census, &report, &manifest)
                .unwrap_err()
                .contains("no specs")
        );

        let temp = tempfile::tempdir().unwrap();
        let (evidence, census, report, manifest) = control_comparison_fixture(temp.path());
        let mut mismatched_manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
        mismatched_manifest["tests"].as_array_mut().unwrap().pop();
        fs::write(&manifest, serde_json::to_vec(&mismatched_manifest).unwrap()).unwrap();
        assert!(
            compare(&evidence, &census, &report, &manifest)
                .unwrap_err()
                .contains("manifest/report")
        );
    }

    #[test]
    fn wrong_lane_and_failed_phase_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let evidence = fixture(temp.path());
        let mut phase: Value =
            serde_json::from_str(&fs::read_to_string(&evidence[0].phase).unwrap()).unwrap();
        phase["lane"] = Value::String("wrong-lane".into());
        fs::write(&evidence[0].phase, serde_json::to_vec(&phase).unwrap()).unwrap();
        assert!(
            errors(&evidence)
                .iter()
                .any(|error| error.contains("phase sidecar"))
        );

        let temp = tempfile::tempdir().unwrap();
        let evidence = fixture(temp.path());
        let mut phase: Value =
            serde_json::from_str(&fs::read_to_string(&evidence[2].phase).unwrap()).unwrap();
        phase["phases"][2]["outcome"] = Value::String("failed".into());
        phase["phases"][2]["detail"] = Value::String("panic".into());
        fs::write(&evidence[2].phase, serde_json::to_vec(&phase).unwrap()).unwrap();
        assert!(
            errors(&evidence)
                .iter()
                .any(|error| error.contains("incomplete or failed"))
        );
    }
}

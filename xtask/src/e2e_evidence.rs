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
        .any(|phase| phase.outcome != PhaseOutcome::Success)
    {
        return Err("records a failed phase".into());
    }
    Ok(())
}

/// Reconcile all candidate lanes. Ownership runs first; every remaining consumer
/// then validates its lane-qualified input independently, retaining no merged or
/// basename-selected evidence.
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
        .filter(|lane| lane.backend == backend && lane.browser == browser && !lane.enabled)
        .map(|lane| lane.identity.as_str())
        .collect::<BTreeSet<_>>();
    if supplied != expected {
        errors.push("lane evidence does not exactly match the catalog".into());
    }
    for item in evidence {
        let Some(lane) = lanes.iter().find(|lane| lane.identity == item.identity) else {
            continue;
        };
        if lane.backend != backend || lane.browser != browser || lane.enabled {
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

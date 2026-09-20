//! Fail-closed reconciliation of an independent E2E census and candidate lanes.

use crate::e2e_lanes::Lane;
use crate::playwright_report::PlaywrightReport;
use serde::Deserialize;
use std::collections::BTreeSet;

type Identity = (String, String, String);
#[derive(Deserialize)]
struct Census {
    schema_version: u8,
    complete: bool,
    topology: String,
    tests: Vec<Test>,
}
#[derive(Deserialize)]
struct Manifest {
    schema_version: u8,
    complete: bool,
    lane: String,
    backend: String,
    browser: String,
    partition: String,
    shard_index: Option<u8>,
    shard_count: Option<u8>,
    tests: Vec<Test>,
}
#[derive(Deserialize)]
struct Test {
    project_id: String,
    project_name: String,
    test_id: String,
}
fn identities(tests: Vec<Test>, label: &str) -> Result<BTreeSet<Identity>, String> {
    let count = tests.len();
    let set = tests
        .into_iter()
        .map(|t| {
            if t.project_id.is_empty() || t.project_name.is_empty() || t.test_id.is_empty() {
                Err(format!("malformed {label} identity"))
            } else {
                Ok((t.project_id, t.project_name, t.test_id))
            }
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if set.len() != count {
        return Err(format!("duplicate {label} identity"));
    }
    Ok(set)
}
fn report_identities(raw: &str, lane: &str) -> Result<BTreeSet<Identity>, String> {
    let report: PlaywrightReport = serde_json::from_str(raw)
        .map_err(|error| format!("malformed report for `{lane}`: {error}"))?;
    let mut specs = Vec::new();
    report.visit_specs(&mut |spec| specs.push(spec));
    if specs.is_empty() {
        return Err(format!("report for `{lane}` has no specs"));
    }
    let mut identities = Vec::new();
    for spec in specs {
        let test_id = spec
            .id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("malformed report test id for `{lane}`"))?;
        for test in &spec.tests {
            let project_id = test
                .project_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("malformed report project id for `{lane}`"))?;
            let project_name = test
                .project_name
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("malformed report project name for `{lane}`"))?;
            identities.push((
                project_id.to_owned(),
                project_name.to_owned(),
                test_id.to_owned(),
            ));
        }
    }
    let count = identities.len();
    let identities = identities.into_iter().collect::<BTreeSet<_>>();
    if identities.len() != count {
        return Err(format!("duplicate report identity for `{lane}`"));
    }
    Ok(identities)
}
/// Validate an unsplit control's complete execution evidence before comparing it
/// with a distributed candidate. The census, manifest, and report must identify
/// exactly the enabled control lane's selected population.
pub(crate) fn validate_unsplit_control(
    backend: &str,
    browser: &str,
    lane: &Lane,
    expected_topology: &str,
    census_raw: &str,
    report_raw: &str,
    manifest_raw: &str,
) -> Result<(), String> {
    if !lane.enabled
        || lane.backend != backend
        || lane.browser != browser
        || lane.partition != "unsplit"
    {
        return Err("control lane is not the enabled unsplit catalog lane".into());
    }
    let census: Census = serde_json::from_str(census_raw)
        .map_err(|error| format!("malformed control census: {error}"))?;
    if census.schema_version != 1 || !census.complete || census.topology != expected_topology {
        return Err("incomplete or mismatched control census".into());
    }
    let expected = identities(census.tests, "control census")?;
    if expected.is_empty() {
        return Err("empty control census".into());
    }
    let manifest: Manifest = serde_json::from_str(manifest_raw)
        .map_err(|error| format!("malformed control lane manifest: {error}"))?;
    if manifest.schema_version != 1
        || !manifest.complete
        || manifest.lane != lane.identity
        || manifest.backend != backend
        || manifest.browser != browser
        || manifest.partition != lane.partition
        || manifest.shard_index != lane.shard_index
        || manifest.shard_count != lane.shard_count
    {
        return Err("mismatched control lane manifest".into());
    }
    let selected = identities(manifest.tests, "control lane manifest")?;
    let reported = report_identities(report_raw, &lane.identity)?;
    if selected != reported {
        return Err("control lane manifest/report population mismatch".into());
    }
    if expected != selected {
        return Err("control census/manifest population mismatch".into());
    }
    Ok(())
}

/// Evidence is `(lane identity, expected census, report, authenticated manifest)`.
pub fn reconcile(
    backend: &str,
    browser: &str,
    candidates: &[Lane],
    evidence: &[(String, String, String, String)],
) -> Result<String, String> {
    let lanes = candidates
        .iter()
        .filter(|l| l.backend == backend && l.browser == browser && !l.enabled)
        .collect::<Vec<_>>();
    let expected_lanes = lanes
        .iter()
        .map(|l| l.identity.as_str())
        .collect::<BTreeSet<_>>();
    let supplied = evidence
        .iter()
        .map(|e| e.0.as_str())
        .collect::<BTreeSet<_>>();
    if lanes.len() != 3 || expected_lanes != supplied || evidence.len() != supplied.len() {
        return Err("candidate lane evidence does not exactly match the catalog".into());
    }
    let mut census_all = None;
    let mut observed = BTreeSet::new();
    for (id, census_raw, report_raw, manifest_raw) in evidence {
        let lane = lanes
            .iter()
            .find(|l| l.identity == *id)
            .ok_or_else(|| format!("unexpected lane `{id}`"))?;
        let census: Census = serde_json::from_str(census_raw)
            .map_err(|e| format!("malformed expected census for `{id}`: {e}"))?;
        if census.schema_version != 1
            || !census.complete
            || census.topology != format!("{backend}-{browser}-experimental")
        {
            return Err(format!(
                "incomplete or mismatched expected census for `{id}`"
            ));
        }
        let expected = identities(census.tests, "expected census")?;
        if expected.is_empty() {
            return Err("empty expected census".into());
        }
        if expected
            .iter()
            .any(|(_, project, _)| !lanes.iter().any(|l| l.projects.contains(project)))
        {
            return Err("expected census has unowned project".into());
        }
        if let Some(prior) = &census_all {
            if prior != &expected {
                return Err("lifted expected censuses are not identical".into());
            }
        } else {
            census_all = Some(expected);
        }
        let manifest: Manifest = serde_json::from_str(manifest_raw)
            .map_err(|e| format!("malformed lane manifest for `{id}`: {e}"))?;
        if manifest.schema_version != 1
            || !manifest.complete
            || manifest.lane != *id
            || manifest.backend != backend
            || manifest.browser != browser
            || manifest.partition != lane.partition
            || manifest.shard_index != lane.shard_index
            || manifest.shard_count != lane.shard_count
        {
            return Err(format!("mismatched lane manifest for `{id}`"));
        }
        let selected = identities(manifest.tests, "lane manifest")?;
        if selected.is_empty()
            || selected
                .iter()
                .any(|(_, project, _)| !lane.projects.contains(project))
        {
            return Err(format!("lane manifest has unowned project for `{id}`"));
        }
        let reported = report_identities(report_raw, id)?;
        if selected != reported {
            return Err(format!(
                "lane manifest/report population mismatch for `{id}`"
            ));
        }
        let prior = observed.len();
        observed.extend(reported);
        if observed.len() != prior + selected.len() {
            return Err(format!("duplicate report identity across lanes for `{id}`"));
        }
    }
    let expected = census_all.expect("evidence checked");
    if expected != observed {
        return Err("global expected census/report union mismatch".into());
    }
    Ok(format!(
        "{backend}/{browser}: {} candidate lanes, {} expected tests",
        lanes.len(),
        expected.len()
    ))
}
#[cfg(test)]
mod tests {
    use super::reconcile;
    use crate::e2e_lanes::catalog;
    const C: &str = include_str!("testdata/e2e-ownership/expected-census.json");
    const R1: &str = include_str!("testdata/e2e-ownership/ordinary-1-report.json");
    const R2: &str = include_str!("testdata/e2e-ownership/ordinary-2-report.json");
    const RS: &str = include_str!("testdata/e2e-ownership/serial-special-report.json");
    const M1: &str = include_str!("testdata/e2e-ownership/ordinary-1-manifest.json");
    const M2: &str = include_str!("testdata/e2e-ownership/ordinary-2-manifest.json");
    const MS: &str = include_str!("testdata/e2e-ownership/serial-special-manifest.json");
    fn e() -> Vec<(String, String, String, String)> {
        vec![
            (
                "sqlite-firefox-ordinary-1-of-2".into(),
                C.into(),
                R1.into(),
                M1.into(),
            ),
            (
                "sqlite-firefox-ordinary-2-of-2".into(),
                C.into(),
                R2.into(),
                M2.into(),
            ),
            (
                "sqlite-firefox-serial-special".into(),
                C.into(),
                RS.into(),
                MS.into(),
            ),
        ]
    }
    #[test]
    fn passing_and_fail_closed_evidence() {
        let lanes = catalog().unwrap().lanes;
        assert!(reconcile("sqlite", "firefox", &lanes, &e()).is_ok());
        let mut x = e();
        x.pop();
        assert!(reconcile("sqlite", "firefox", &lanes, &x).is_err());
        let mut x = e();
        x[0].3 = x[1].3.clone();
        assert!(reconcile("sqlite", "firefox", &lanes, &x).is_err());
        let mut x = e();
        x[0].2 = x[1].2.clone();
        assert!(reconcile("sqlite", "firefox", &lanes, &x).is_err());
        let mut x = e();
        x[0].1 = "{".into();
        assert!(reconcile("sqlite", "firefox", &lanes, &x).is_err());
        let mut x = e();
        x[0].3 = "{".into();
        assert!(reconcile("sqlite", "firefox", &lanes, &x).is_err());
    }
}

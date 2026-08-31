//! Strict host-side validation of Playwright projects against trace boot evidence.
//! The lifted Playwright report is the sole authority for the executed project
//! population, under
//! `docs/adr/drafts/playwright-report-defines-trace-gate-population.md`.
//! Evidence is accepted from `e2e.test` and navigation-bearing `e2e.page` spans,
//! reconciled exactly to that report, and uses the analyzer's non-null
//! `commitToMountMs` proxy only to define mounted membership. Every mounted
//! navigation must use the current schema, contain a complete document-frame
//! decomposition, and close to its document-frame target within 1 ms; no
//! navigation may have been dropped. Nix invokes this only after a successful
//! combination's diagnostics are lifted, after the duration validator, so a VM
//! failure remains its own primary result.

use std::collections::{BTreeMap, BTreeSet};

use std::path::Path;

use serde_json::Value;

use crate::playwright_report::PlaywrightReport;
use crate::traces::run::read_trace_member;

use crate::StepResult;
use crate::traces::{
    boot_phases::{
        BootDecompositionOutcome, boot_decomposition_outcome, page_boot_decomposition_outcome,
    },
    parse::{Filters, get_attr, parse_json_attr, parse_spans},
};

/// Validate one successful E2E combination's independently lifted execution
/// population and trace evidence.
///
/// The Nix orchestration calls this only after lifting diagnostics and confirming
/// the VM itself passed, preserving its primary failure if Playwright failed.
pub(crate) fn validate_lifted_combo(backend: &str, browser: &str) -> StepResult {
    let diagnostics = Path::new(".xtask/diagnostics").join(format!("e2e-{backend}-{browser}"));
    let report = diagnostics.join(format!("playwright-report-{backend}.json"));
    let capture = diagnostics.join(format!("capture-{backend}.tar.gz"));

    match validate_files(&report, &capture) {
        Ok(detail) => StepResult::ok("e2e-boot-decomposition-coverage").detail(detail),
        Err(error) => StepResult::fail("e2e-boot-decomposition-coverage").detail(format!(
            "e2e {backend}/{browser} boot-decomposition evidence is unavailable or inconsistent: {error}"
        )),
    }
}

fn validate_files(report_path: &Path, capture_path: &Path) -> Result<String, String> {
    let report = std::fs::read_to_string(report_path).map_err(|error| {
        format!(
            "reading Playwright report {}: {error}",
            report_path.display()
        )
    })?;
    if report.trim().is_empty() {
        return Err(format!(
            "Playwright report {} is empty",
            report_path.display()
        ));
    }
    let trace = read_trace_member(capture_path)
        .map_err(|error| format!("reading capture {}: {error:#}", capture_path.display()))?;
    validate_json(&report, &trace)
}

/// Evaluate in-memory artifacts at the stable host-side test seam.
pub(crate) fn validate_json(report_json: &str, trace_jsonl: &str) -> Result<String, String> {
    let report: PlaywrightReport = serde_json::from_str(report_json)
        .map_err(|error| format!("parsing Playwright report: {error}"))?;
    let expected = report_projects(&report)?;
    let spans = parse_spans(trace_jsonl, &Filters::default(), "trace capture")
        .map_err(|error| format!("parsing trace capture: {error:#}"))?;
    let observed = trace_projects(&spans)?;

    if expected != observed {
        let missing = expected.difference(&observed).collect::<Vec<_>>();
        let unexpected = observed.difference(&expected).collect::<Vec<_>>();
        return Err(format!(
            "project-set mismatch: missing trace projects [{}]; unexpected trace projects [{}]",
            names(missing),
            names(unexpected)
        ));
    }

    Ok(format!(
        "boot-decomposition evidence complete for {} project(s)",
        expected.len()
    ))
}

fn report_projects(report: &PlaywrightReport) -> Result<BTreeSet<String>, String> {
    let mut projects = BTreeSet::new();
    report.visit_specs(&mut |spec| {
        projects.extend(
            spec.tests
                .iter()
                .filter_map(|test| test.project_name.clone()),
        );
    });
    if projects.is_empty() || projects.iter().any(|project| project.is_empty()) {
        return Err("Playwright report has no complete project population".into());
    }
    Ok(projects)
}

fn trace_projects(spans: &[crate::traces::parse::Span]) -> Result<BTreeSet<String>, String> {
    let mut projects = BTreeSet::new();
    let mut mounted = BTreeMap::<String, usize>::new();

    for span in spans
        .iter()
        .filter(|span| span.name == "e2e.test" || span.name == "e2e.page")
    {
        if span.project.is_empty() {
            return Err(format!("{} span has no e2e.project", span.name));
        }
        let project = &span.project;
        projects.insert(project.clone());
        validate_dropped(span, project)?;

        let navigations = parse_json_attr(&span.raw, "e2e.navigation_top_json", &span.source)
            .map_err(|error| {
                format!("project {project}: malformed navigation evidence: {error:#}")
            })?
            .ok_or_else(|| format!("project {project}: missing navigation evidence"))?;
        let navigations = navigations
            .as_array()
            .ok_or_else(|| format!("project {project}: navigation evidence is not an array"))?;
        let marks =
            parse_json_attr(&span.raw, "e2e.boot_marks_json", &span.source).map_err(|error| {
                format!("project {project}: malformed boot-mark evidence: {error:#}")
            })?;
        let marks = marks_by_navigation(marks.as_ref(), project)?;
        for navigation in navigations {
            let object = navigation
                .as_object()
                .ok_or_else(|| format!("project {project}: navigation entry is not an object"))?;
            match object.get("commitToMountMs") {
                Some(Value::Null) => continue,
                Some(Value::Number(value)) if value.as_f64().is_some_and(f64::is_finite) => {}
                Some(_) => {
                    return Err(format!(
                        "project {project}: navigation commitToMountMs is not null or a finite number"
                    ));
                }
                None => {
                    return Err(format!(
                        "project {project}: navigation evidence has no commitToMountMs"
                    ));
                }
            }
            *mounted.entry(project.clone()).or_default() += 1;
            validate_navigation(project, navigation, &marks, span.name == "e2e.page")?;
        }
    }

    for project in &projects {
        if mounted.get(project).copied().unwrap_or_default() == 0 {
            return Err(format!("project {project}: no mounted navigation evidence"));
        }
    }
    if projects.is_empty() {
        return Err("trace capture has no navigation-bearing e2e.test or e2e.page spans".into());
    }
    Ok(projects)
}

fn has_attribute(raw: &Value, key: &str) -> bool {
    raw.get("attributes")
        .and_then(Value::as_array)
        .is_some_and(|attributes| {
            attributes
                .iter()
                .any(|attribute| attribute.get("key").and_then(Value::as_str) == Some(key))
        })
}

fn validate_dropped(span: &crate::traces::parse::Span, project: &str) -> Result<(), String> {
    if !has_attribute(&span.raw, "e2e.navigation_top_dropped") {
        return Err(format!(
            "project {project}: missing dropped-navigation count"
        ));
    }
    let value = get_attr(&span.raw, "e2e.navigation_top_dropped");
    let dropped = value
        .parse::<u64>()
        .map_err(|_| format!("project {project}: malformed dropped-navigation count {value:?}"))?;
    if dropped == 0 {
        Ok(())
    } else {
        Err(format!(
            "project {project}: {dropped} dropped navigation record(s)"
        ))
    }
}

fn marks_by_navigation<'a>(
    marks: Option<&'a Value>,
    project: &str,
) -> Result<BTreeMap<i64, &'a [Value]>, String> {
    let Some(marks) = marks else {
        return Ok(BTreeMap::new());
    };
    let marks = marks
        .as_array()
        .ok_or_else(|| format!("project {project}: boot-mark evidence is not an array"))?;
    let mut by_navigation = BTreeMap::new();
    for entry in marks {
        let entry = entry
            .as_object()
            .ok_or_else(|| format!("project {project}: boot-mark entry is not an object"))?;
        let id = entry
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("project {project}: boot-mark entry has no integer id"))?;
        let marks = entry
            .get("marks")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("project {project}: boot-mark entry has no marks array"))?;
        for mark in marks {
            let mark = mark
                .as_object()
                .ok_or_else(|| format!("project {project}: boot-mark record is not an object"))?;
            if mark.get("name").and_then(Value::as_str).is_none() {
                return Err(format!(
                    "project {project}: boot-mark record has no string name"
                ));
            }
            if !mark
                .get("startTime")
                .and_then(Value::as_f64)
                .is_some_and(f64::is_finite)
            {
                return Err(format!(
                    "project {project}: boot-mark startTime is not a finite number"
                ));
            }
        }
        if by_navigation.insert(id, marks.as_slice()).is_some() {
            return Err(format!(
                "project {project}: duplicate boot-mark navigation id {id}"
            ));
        }
    }
    Ok(by_navigation)
}

fn validate_navigation(
    project: &str,
    navigation: &Value,
    marks: &BTreeMap<i64, &[Value]>,
    page_only: bool,
) -> Result<(), String> {
    if navigation.get("wasmTimingSchema").and_then(Value::as_str) != Some("direct-init-v1") {
        return Err(format!(
            "project {project}: legacy or missing timing schema"
        ));
    }
    // The shared decomposition seam owns both the finite boot-interval floor and
    // document-frame closure, so the gate cannot count unrelated properties.
    let id = navigation.get("id").and_then(Value::as_i64);
    let marks = id
        .and_then(|id| marks.get(&id))
        .copied()
        .unwrap_or_default();
    let outcome = if page_only {
        page_boot_decomposition_outcome(navigation, marks)
    } else {
        boot_decomposition_outcome(navigation, marks)
    };
    match outcome {
        BootDecompositionOutcome::Complete => Ok(()),
        BootDecompositionOutcome::Incomplete => Err(format!(
            "project {project}: incomplete direct-init decomposition"
        )),
        BootDecompositionOutcome::ClosureViolation => Err(format!(
            "project {project}: document-frame decomposition does not close within 1 ms"
        )),
    }
}

fn names(names: Vec<&String>) -> String {
    names
        .into_iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::{Value, json};

    use super::{validate_files, validate_json};
    use crate::traces::run::TRACE_MEMBER;

    fn report(projects: &[&str]) -> String {
        json!({
            "suites": [{
                "specs": [{
                    "tests": projects.iter().map(|project| json!({
                        "projectName": project,
                    })).collect::<Vec<_>>(),
                }],
            }],
        })
        .to_string()
    }

    fn attr(key: &str, value: Value) -> Value {
        json!({ "key": key, "value": { "stringValue": value.to_string() } })
    }

    fn string_attr(key: &str, value: &str) -> Value {
        json!({ "key": key, "value": { "stringValue": value } })
    }

    fn navigation(id: i64, complete: bool) -> Value {
        json!({
            "id": id,
            "wasmTimingSchema": "direct-init-v1",
            "commitToMountMs": 42.0,
            "wasmInitStartMs": 5.0,
            "wasmInitStartToBootEntryMs": 5.0,
            "bootPhases": if complete {
                json!({
                    "jaunder.boot.entry->jaunder.boot.seed_parsed": 10.0,
                    "jaunder.boot.seed_parsed->jaunder.boot.render_start": 15.0,
                    "jaunder.boot.render_start->jaunder.boot.mount_done": 20.0,
                })
            } else {
                json!({ "jaunder.boot.entry->jaunder.boot.mount_done": 40.0 })
            },
        })
    }

    fn marks(id: i64) -> Value {
        json!({
            "id": id,
            "marks": [
                { "name": "jaunder.boot.entry", "startTime": 10.0 },
                { "name": "jaunder.boot.seed_parsed", "startTime": 20.0 },
                { "name": "jaunder.boot.render_start", "startTime": 35.0 },
                { "name": "jaunder.boot.mount_done", "startTime": 55.0 },
            ],
        })
    }

    fn trace(project: &str, span_name: &str, nav: Value, marks: Value, dropped: u64) -> String {
        json!({
            "resourceSpans": [{ "scopeSpans": [{ "spans": [{
                "name": span_name,
                "attributes": [
                    string_attr("e2e.project", project),
                    attr("e2e.navigation_top_json", json!([nav])),
                    attr("e2e.boot_marks_json", json!([marks])),
                    attr("e2e.navigation_top_dropped", json!(dropped)),
                ],
            }] }] }],
        })
        .to_string()
    }

    fn valid_trace() -> String {
        trace("chromium", "e2e.test", navigation(1, true), marks(1), 0)
    }

    fn malformed_navigation_trace() -> String {
        let mut record: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes = record["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
            .as_array_mut()
            .unwrap();
        let navigation = attributes
            .iter_mut()
            .find(|attribute| attribute["key"].as_str() == Some("e2e.navigation_top_json"))
            .unwrap();
        navigation["value"]["stringValue"] = json!("{");
        record.to_string()
    }

    fn trace_without_marks(project: &str, span_name: &str, nav: Value) -> String {
        json!({
            "resourceSpans": [{ "scopeSpans": [{ "spans": [{
                "name": span_name,
                "attributes": [
                    string_attr("e2e.project", project),
                    attr("e2e.navigation_top_json", json!([nav])),
                    attr("e2e.navigation_top_dropped", json!(0)),
                ],
            }] }] }],
        })
        .to_string()
    }

    fn page_navigation_with_document_total(id: i64, document_boot_total_ms: f64) -> Value {
        let mut nav = navigation(id, true);
        nav["documentBootTotalMs"] = json!(document_boot_total_ms);
        nav
    }

    fn assert_rejected(report_json: &str, trace_jsonl: &str, expected: &str) {
        let error = validate_json(report_json, trace_jsonl).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }

    fn write_capture(path: &Path, traces: &[&str]) {
        let file = fs::File::create(path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for trace in traces {
            let mut header = tar::Header::new_gnu();
            header.set_size(trace.len().try_into().unwrap());
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, TRACE_MEMBER, trace.as_bytes())
                .unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn validates_lifted_report_and_exactly_one_nonempty_trace_member() {
        let temp = tempfile::tempdir().unwrap();
        let report_path = temp.path().join("playwright-report-sqlite.json");
        let capture_path = temp.path().join("capture-sqlite.tar.gz");
        fs::write(&report_path, report(&["chromium"])).unwrap();
        write_capture(&capture_path, &[&valid_trace()]);
        assert!(validate_files(&report_path, &capture_path).is_ok());

        fs::write(&report_path, "").unwrap();
        let error = validate_files(&report_path, &capture_path).unwrap_err();
        assert!(
            error.contains("Playwright report") && error.contains("empty"),
            "{error}"
        );
        fs::write(&report_path, report(&["chromium"])).unwrap();

        fs::remove_file(&capture_path).unwrap();
        let error = validate_files(&report_path, &capture_path).unwrap_err();
        assert!(error.contains("opening capture"), "{error}");

        fs::write(&capture_path, b"not a gzip archive").unwrap();
        let error = validate_files(&report_path, &capture_path).unwrap_err();
        assert!(error.contains("reading capture"), "{error}");

        write_capture(&capture_path, &[]);
        let error = validate_files(&report_path, &capture_path).unwrap_err();
        assert!(error.contains("missing"), "{error}");

        write_capture(&capture_path, &[""]);
        let error = validate_files(&report_path, &capture_path).unwrap_err();
        assert!(error.contains("empty"), "{error}");

        let trace = valid_trace();
        write_capture(&capture_path, &[&trace, &trace]);
        let error = validate_files(&report_path, &capture_path).unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");
    }

    #[test]
    fn rejects_missing_empty_and_malformed_artifacts() {
        assert_rejected("", &valid_trace(), "parsing Playwright report");
        assert_rejected(
            r#"{"suites":[]}"#,
            &valid_trace(),
            "no complete project population",
        );
        assert_rejected(&report(&["chromium"]), "", "no navigation-bearing");
        assert_rejected(&report(&["chromium"]), "{", "parsing trace capture");
        assert_rejected(
            &report(&["chromium"]),
            &malformed_navigation_trace(),
            "malformed navigation evidence",
        );
    }

    #[test]
    fn rejects_present_navigation_evidence_that_is_not_an_array() {
        let mut record: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes = record["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
            .as_array_mut()
            .unwrap();
        let navigation = attributes
            .iter_mut()
            .find(|attribute| attribute["key"].as_str() == Some("e2e.navigation_top_json"))
            .unwrap();
        navigation["value"]["stringValue"] = json!(json!({"id": 1}).to_string());
        assert_rejected(
            &report(&["chromium"]),
            &record.to_string(),
            "navigation evidence is not an array",
        );
    }

    #[test]
    fn rejects_missing_required_navigation_and_dropped_evidence() {
        let mut missing_navigation: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes =
            missing_navigation["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
                .as_array_mut()
                .unwrap();
        attributes.retain(|attribute| attribute["key"].as_str() != Some("e2e.navigation_top_json"));
        assert_rejected(
            &report(&["chromium"]),
            &missing_navigation.to_string(),
            "missing navigation evidence",
        );
        missing_navigation["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"] =
            json!("e2e.page");
        assert_rejected(
            &report(&["chromium"]),
            &missing_navigation.to_string(),
            "missing navigation evidence",
        );

        let mut missing_dropped: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes =
            missing_dropped["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
                .as_array_mut()
                .unwrap();
        attributes
            .retain(|attribute| attribute["key"].as_str() != Some("e2e.navigation_top_dropped"));
        assert_rejected(
            &report(&["chromium"]),
            &missing_dropped.to_string(),
            "missing dropped-navigation count",
        );

        let mut negative_dropped: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes =
            negative_dropped["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
                .as_array_mut()
                .unwrap();
        let dropped = attributes
            .iter_mut()
            .find(|attribute| attribute["key"].as_str() == Some("e2e.navigation_top_dropped"))
            .unwrap();
        dropped["value"]["stringValue"] = json!("-1");
        assert_rejected(
            &report(&["chromium"]),
            &negative_dropped.to_string(),
            "malformed dropped-navigation count",
        );
    }

    #[test]
    fn rejects_malformed_and_duplicate_boot_mark_evidence() {
        let mut not_an_array: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes =
            not_an_array["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
                .as_array_mut()
                .unwrap();
        let boot_marks = attributes
            .iter_mut()
            .find(|attribute| attribute["key"].as_str() == Some("e2e.boot_marks_json"))
            .unwrap();
        boot_marks["value"]["stringValue"] = json!(json!({"id": 1}).to_string());
        assert_rejected(
            &report(&["chromium"]),
            &not_an_array.to_string(),
            "boot-mark evidence is not an array",
        );

        let mut duplicate: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes = duplicate["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
            .as_array_mut()
            .unwrap();
        let boot_marks = attributes
            .iter_mut()
            .find(|attribute| attribute["key"].as_str() == Some("e2e.boot_marks_json"))
            .unwrap();
        boot_marks["value"]["stringValue"] = json!(json!([marks(1), marks(1)]).to_string());
        assert_rejected(
            &report(&["chromium"]),
            &duplicate.to_string(),
            "duplicate boot-mark navigation id",
        );

        let mut malformed_mark: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes =
            malformed_mark["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
                .as_array_mut()
                .unwrap();
        let boot_marks = attributes
            .iter_mut()
            .find(|attribute| attribute["key"].as_str() == Some("e2e.boot_marks_json"))
            .unwrap();
        boot_marks["value"]["stringValue"] = json!(
            json!([{ "id": 1, "marks": [{ "name": "jaunder.boot.entry", "startTime": "early" }] }])
                .to_string()
        );
        assert_rejected(
            &report(&["chromium"]),
            &malformed_mark.to_string(),
            "boot-mark startTime is not a finite number",
        );

        let mut missing_id: Value = serde_json::from_str(&valid_trace()).unwrap();
        let attributes = missing_id["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"]
            .as_array_mut()
            .unwrap();
        let boot_marks = attributes
            .iter_mut()
            .find(|attribute| attribute["key"].as_str() == Some("e2e.boot_marks_json"))
            .unwrap();
        boot_marks["value"]["stringValue"] = json!(json!([{ "marks": [] }]).to_string());
        assert_rejected(
            &report(&["chromium"]),
            &missing_id.to_string(),
            "boot-mark entry has no integer id",
        );
    }

    #[test]
    fn rejects_mismatched_project_sets() {
        assert_rejected(
            &report(&["chromium", "firefox"]),
            &valid_trace(),
            "project-set mismatch",
        );
    }

    #[test]
    fn rejects_navigation_missing_mounted_membership_field() {
        let mut nav = navigation(1, true);
        nav.as_object_mut().unwrap().remove("commitToMountMs");
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", nav, marks(1), 0),
            "no commitToMountMs",
        );
    }

    #[test]
    fn accepts_null_membership_as_unmounted_and_rejects_wrong_navigation_entries() {
        let mut unmounted = navigation(1, true);
        unmounted["commitToMountMs"] = Value::Null;
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", unmounted, marks(1), 0),
            "no mounted navigation",
        );

        let mut wrong = navigation(1, true);
        wrong["commitToMountMs"] = json!("fast");
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", wrong, marks(1), 0),
            "not null or a finite number",
        );

        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", json!(false), marks(1), 0),
            "navigation entry is not an object",
        );
    }

    #[test]
    fn rejects_legacy_and_partial_navigation_evidence() {
        let mut legacy = navigation(1, true);
        legacy["wasmTimingSchema"] = json!("legacy-v1");
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", legacy, marks(1), 0),
            "legacy or missing timing schema",
        );
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", navigation(1, false), marks(1), 0),
            "incomplete direct-init decomposition",
        );
    }

    #[test]
    fn rejects_junk_properties_that_do_not_supply_boot_phase_intervals() {
        let mut nav = navigation(1, true);
        nav["bootPhases"] = json!({
            "jaunder.boot.entry->jaunder.boot.mount_done": 40.0,
            "notBoot": 1.0,
            "boot.not_an_interval": "not finite",
            "diagnostic": 3.0,
        });
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", nav, marks(1), 0),
            "incomplete direct-init decomposition",
        );
    }

    #[test]
    fn rejects_missing_direct_init_fields() {
        let mut nav = navigation(1, true);
        nav.as_object_mut().unwrap().remove("wasmInitStartMs");
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", nav, marks(1), 0),
            "incomplete direct-init decomposition",
        );
    }

    #[test]
    fn rejects_nonclosing_test_navigation() {
        let mut nav = navigation(1, true);
        nav["bootPhases"]["jaunder.boot.render_start->jaunder.boot.mount_done"] = json!(22.0);
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", nav, marks(1), 0),
            "does not close within 1 ms",
        );
    }

    #[test]
    fn rejects_nonclosing_page_navigation() {
        let mut nav = navigation(1, true);
        nav["bootPhases"]["jaunder.boot.render_start->jaunder.boot.mount_done"] = json!(22.0);
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.page", nav, marks(1), 0),
            "does not close within 1 ms",
        );
    }

    #[test]
    fn rejects_test_span_document_total_without_boot_marks() {
        let trace = trace_without_marks(
            "chromium",
            "e2e.test",
            page_navigation_with_document_total(1, 55.0),
        );

        assert_rejected(
            &report(&["chromium"]),
            &trace,
            "incomplete direct-init decomposition",
        );
    }

    #[test]
    fn accepts_page_navigation_closed_to_its_navigation_document_total_without_marks() {
        let trace = trace_without_marks(
            "chromium",
            "e2e.page",
            page_navigation_with_document_total(1, 55.0),
        );

        validate_json(&report(&["chromium"]), &trace).unwrap();
    }

    #[test]
    fn rejects_page_navigation_not_closed_to_its_navigation_document_total_without_marks() {
        let trace = trace_without_marks(
            "chromium",
            "e2e.page",
            page_navigation_with_document_total(1, 56.5),
        );

        assert_rejected(&report(&["chromium"]), &trace, "does not close within 1 ms");
    }

    #[test]
    fn rejects_dropped_navigation_evidence() {
        assert_rejected(
            &report(&["chromium"]),
            &trace("chromium", "e2e.test", navigation(1, true), marks(1), 1),
            "dropped navigation record",
        );
    }
    #[test]
    fn accepts_complete_current_evidence_for_every_reported_project() {
        let trace = format!(
            "{}\n{}",
            trace("chromium", "e2e.test", navigation(1, true), marks(1), 0),
            trace("firefox", "e2e.page", navigation(2, true), marks(2), 0),
        );

        validate_json(&report(&["chromium", "firefox"]), &trace).unwrap();
    }
}

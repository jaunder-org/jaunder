//! The three trace-analysis commands as testable report-returning functions.

use std::path::PathBuf;
use std::time::Instant;

use super::{analyze, boot_phases, parse, render, report, run};
use crate::e2e::E2eBrowser;
use crate::report::{CommandResult, StepResult};

/// `trace-analysis analyze`: tables over exported `otel-traces.jsonl` files.
pub fn analyze_command(
    top: u64,
    trace: Option<String>,
    project: Option<String>,
    playwright_report: &[PathBuf],
    files: &[PathBuf],
) -> anyhow::Result<CommandResult> {
    let start = Instant::now();
    let mut result = CommandResult::new("traces-analyze");
    let filters = parse::Filters { trace, project };
    let reported = report::ReportedDurations::from_paths(playwright_report)?;
    if let Some(analysis) = attribute_owner_result(
        &mut result,
        "traces-analyze",
        analyze::analyze(files, filters, &reported),
    )? {
        result.report = Some(render::render(&analysis, top as usize));
        result.push(
            StepResult::ok("traces-analyze")
                .detail(format!("{} span(s)", analysis.span_count))
                .with_duration(start.elapsed()),
        );
    }
    result.duration_ms = start.elapsed().as_millis();
    Ok(result)
}

/// `trace-analysis run`: build the e2e checks, then analyze their traces.
pub fn run_command(
    top: u64,
    trace: Option<String>,
    single_worker: bool,
    browser: Option<E2eBrowser>,
) -> anyhow::Result<CommandResult> {
    let start = Instant::now();
    let mut result = CommandResult::new("traces-run");
    // `_tmp` guards extracted traces until analysis ends. Collection and Nix
    // failures retain the trace command's top-level error contract.
    let (_tmp, files, reports) = run::collect_trace_files(single_worker, browser)?;
    let filters = parse::Filters {
        trace,
        project: None,
    };
    // Paired per combo — see `ReportedDurations`: sqlite and postgres share
    // test+project+retry keys, so an unpaired merge would let one backend's
    // durations overwrite the other's.
    let reported = report::ReportedDurations::from_labeled(&reports)?;
    if let Some(analysis) = attribute_owner_result(
        &mut result,
        "traces-run",
        analyze::analyze(&files, filters, &reported),
    )? {
        result.report = Some(render::render(&analysis, top as usize));
        result.push(
            StepResult::ok("traces-run")
                .detail(format!("{} trace file(s)", files.len()))
                .with_duration(start.elapsed()),
        );
    }
    result.duration_ms = start.elapsed().as_millis();
    Ok(result)
}

/// `trace-analysis boot-phases`: median boot-phase decomposition (#818).
pub fn boot_phases_command(files: &[PathBuf]) -> anyhow::Result<CommandResult> {
    let start = Instant::now();
    let mut result = CommandResult::new("traces-boot-phases");
    if let Some(rows) = attribute_owner_result(
        &mut result,
        "traces-boot-phases",
        boot_phases::boot_phases(files),
    )? {
        result.report = Some(boot_phases::render(&rows));
        result.push(
            StepResult::ok("traces-boot-phases")
                .detail(format!("{} population(s)", rows.len()))
                .with_duration(start.elapsed()),
        );
    }
    result.duration_ms = start.elapsed().as_millis();
    Ok(result)
}

/// A malformed span attribute is the analysis's own failed verdict; every other
/// failure (missing files, Nix) stays a top-level error.
fn attribute_owner_result<T>(
    result: &mut CommandResult,
    step: &'static str,
    value: anyhow::Result<T>,
) -> anyhow::Result<Option<T>> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.downcast_ref::<parse::MalformedJsonAttr>().is_some() => {
            result.push(StepResult::fail(step).detail(format!("{error:#}")));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    #[test]
    fn missing_trace_file_remains_a_top_level_error() {
        let path = "/no/such/trace.jsonl";
        let error = analyze_command(25, None, None, &[], &[PathBuf::from(path)]).unwrap_err();
        assert!(format!("{error:#}").contains(path));
    }

    #[test]
    fn malformed_attribute_is_a_serializable_failed_report() {
        let span = serde_json::json!({
            "attributes": [{ "key": "e2e.x", "value": { "stringValue": "{not json" } }]
        });
        let error = parse::parse_json_attr(&span, "e2e.x", "source.jsonl").unwrap_err();
        let mut result = CommandResult::new("traces-analyze");
        let value: Option<()> =
            attribute_owner_result(&mut result, "traces-analyze", Err(error)).unwrap();
        assert!(value.is_none());
        assert_eq!(result.exit_code(), 1);
        let json = serde_json::to_string(&result).unwrap();
        for needle in ["source.jsonl", "e2e.x", "traces-analyze"] {
            assert!(json.contains(needle), "{json}");
        }
    }

    #[test]
    fn non_attribute_failures_remain_top_level_errors() {
        let mut result = CommandResult::new("traces-run");
        let error = attribute_owner_result::<()>(
            &mut result,
            "traces-run",
            Err(anyhow::anyhow!("nix build failed")),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("nix build failed"));
        assert!(result.steps.is_empty());
    }

    #[test]
    fn analyze_renders_tables_into_the_report_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("otel-traces.jsonl");
        std::fs::write(&path, include_str!("testdata/otel-traces-sample.jsonl")).unwrap();
        let result = analyze_command(5, None, Some("firefox".into()), &[], &[path]).unwrap();
        assert!(result.ok, "{:?}", result.steps);
        assert!(
            result
                .report
                .as_deref()
                .is_some_and(|text| !text.is_empty())
        );
    }
}

use std::path::PathBuf;

use anyhow::Result;

use super::super::{
    parse::{Filters, Span, read_spans},
    report::ReportedDurations,
};
use super::model::Analysis;
use super::{browser, span_tree, summary};

/// Compute the whole [`Analysis`] from already-parsed spans. No I/O.
///
/// `reported` supplies the span-coverage section's denominator — Playwright's own
/// per-test wall-clock, which does not live in the traces. Pass
/// `&ReportedDurations::default()` when there is no report; the section then
/// carries a note saying so rather than rendering as fully-covered.
pub fn analyze_spans(
    spans: Vec<Span>,
    project_filter: Option<String>,
    reported: &ReportedDurations,
) -> Result<Analysis> {
    let coverage = super::span_coverage(&spans, reported);
    let coverage_note = span_tree::coverage_note(&spans, reported, &coverage);
    let slowest_spans = summary::slowest_spans(&spans);
    let slowest_e2e_tests = summary::slowest_e2e_tests(&spans);
    let by_project = summary::by_project_rows(&spans);
    let trace_totals = summary::trace_total_rows(&spans);
    let action_hotspots = browser::action_hotspot_rows(&spans)?;
    let (navigation_phase_hotspots, navigation_targets) = browser::navigation_sections(&spans)?;
    let boot_coverage = browser::boot_coverage_rows(&spans)?;
    let (long_task_hotspots, long_task_by_project) = browser::long_task_sections(&spans)?;
    let (resource_initiators, resource_assets) = browser::resource_sections(&spans)?;

    Ok(Analysis {
        span_count: spans.len(),
        project_filter,
        slowest_spans,
        slowest_e2e_tests,
        by_project,
        trace_totals,
        action_hotspots,
        boot_coverage,
        navigation_phase_hotspots,
        navigation_targets,
        long_task_hotspots,
        long_task_by_project,
        resource_initiators,
        resource_assets,
        span_coverage: coverage,
        span_coverage_note: coverage_note,
    })
}

/// Read + parse every input, then analyze. `filters.project` is carried into
/// `Analysis.project_filter` for the render header.
///
/// `reports` are Playwright `json` reporter outputs supplying the span-coverage
/// section's denominator. Empty is fine — the section then renders a note saying
/// why it is absent rather than silently omitting itself.
pub fn analyze(
    inputs: &[PathBuf],
    filters: Filters,
    reported: &ReportedDurations,
) -> Result<Analysis> {
    let mut spans = Vec::new();
    for input in inputs {
        spans.extend(read_spans(input, &filters)?);
    }
    super::analyze_spans(spans, filters.project, reported)
}

#[cfg(test)]
mod tests {
    use super::{analyze, analyze_spans};
    use crate::traces::parse::{Filters, parse_spans};
    use crate::traces::report::ReportedDurations;

    const FIXTURE: &str = include_str!("../testdata/otel-traces-sample.jsonl");

    #[test]
    fn analyze_project_filter_over_fixture() {
        // §8: exercise a `--project` run and the e2e-only filter end-to-end over
        // the committed fixture.
        let filters = Filters {
            trace: None,
            project: Some("firefox".into()),
        };
        let spans = parse_spans(FIXTURE, &filters, "sample").unwrap();
        let a = analyze_spans(
            spans,
            filters.project.clone(),
            &ReportedDurations::default(),
        )
        .unwrap();
        assert_eq!(a.project_filter.as_deref(), Some("firefox"));
        // Only the firefox e2e.test survives; the chromium one is filtered out.
        assert_eq!(a.slowest_e2e_tests.len(), 1);
        assert_eq!(a.slowest_e2e_tests[0].project, "firefox");
        // HTTP spans always pass the project filter (both traces' GET/POST remain).
        assert!(a.slowest_spans.iter().any(|r| r.name == "GET"));
        assert!(a.slowest_spans.iter().any(|r| r.name == "POST"));
    }

    #[test]
    fn trace_json_attr_analyze_fails_on_malformed_present_value() {
        let dir = std::env::temp_dir().join(format!("traces-analyze-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("otel-traces.jsonl");
        std::fs::write(&file, FIXTURE).unwrap();

        let error =
            analyze(&[file], Filters::default(), &ReportedDurations::default()).unwrap_err();
        std::fs::remove_dir_all(&dir).ok();

        let detail = format!("{error:#}");
        assert!(detail.contains("e2e.navigation_top_json"), "{detail}");
        assert!(detail.contains("otel-traces.jsonl"), "{detail}");
        assert!(
            error
                .downcast_ref::<crate::traces::parse::MalformedJsonAttr>()
                .is_some()
        );
    }
}

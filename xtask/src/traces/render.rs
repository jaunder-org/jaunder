//! Render an [`Analysis`](super::analyze::Analysis) to human-facing report tables.
//!
//! The display layer: it applies `--top` slicing and formats numbers, then lays
//! the rows out with `tabled` (no hand-rolled column padding — ADR-0028 cycle
//! decision). `analyze` already fully sorted every section; ranked tables are
//! sliced to `top` here, the all-row tables (added in later tasks) are not.

use tabled::settings::Style;
use tabled::{Table, Tabled};

use super::analyze::{
    Analysis, AssetRow, ByProjectRow, E2eTestRow, HotspotRow, LongTaskProjectRow, SlowSpanRow,
    TargetRow, TraceTotalRow,
};

/// Format a millisecond value the way the Node reports did — three decimals.
fn ms(value: f64) -> String {
    format!("{value:.3}")
}

/// A string column: empty renders as `-` (Node's `value || "-"`).
fn dash(value: &str) -> String {
    if value.is_empty() {
        "-".to_string()
    } else {
        value.to_string()
    }
}

/// Lay out `rows` as a titled table. Empty input renders nothing (the caller
/// decides whether a section is always-on or skip-when-empty).
fn section<T: Tabled>(out: &mut String, title: &str, rows: &[T]) {
    if rows.is_empty() {
        return;
    }
    out.push_str(title);
    out.push('\n');
    out.push_str(&Table::new(rows).with(Style::sharp()).to_string());
    out.push_str("\n\n");
}

#[derive(Tabled)]
struct SlowSpanDisplay {
    duration_ms: String,
    trace_id: String,
    span: String,
    method: String,
    uri: String,
    busy_ns: String,
    idle_ns: String,
    source: String,
}

impl From<&SlowSpanRow> for SlowSpanDisplay {
    fn from(r: &SlowSpanRow) -> Self {
        Self {
            duration_ms: ms(r.duration_ms),
            trace_id: dash(&r.trace_id),
            span: dash(&r.name),
            method: dash(&r.method),
            uri: dash(&r.uri),
            busy_ns: dash(&r.busy_ns),
            idle_ns: dash(&r.idle_ns),
            source: r.source.clone(),
        }
    }
}

/// Take the first `top` of `rows` as display structs (ranked tables).
fn top_display<T, D: for<'a> From<&'a T>>(rows: &[T], top: usize) -> Vec<D> {
    rows.iter().take(top).map(Into::into).collect()
}

/// All `rows` as display structs (the tables that ignore `--top`).
fn all_display<T, D: for<'a> From<&'a T>>(rows: &[T]) -> Vec<D> {
    rows.iter().map(Into::into).collect()
}

#[derive(Tabled)]
struct E2eTestDisplay {
    duration_ms: String,
    project: String,
    actions: u64,
    requests: u64,
    trace_id: String,
    test: String,
}

impl From<&E2eTestRow> for E2eTestDisplay {
    fn from(r: &E2eTestRow) -> Self {
        Self {
            duration_ms: ms(r.duration_ms),
            project: r.project.clone(),
            actions: r.actions,
            requests: r.requests,
            trace_id: dash(&r.trace_id),
            test: r.test.clone(),
        }
    }
}

#[derive(Tabled)]
struct ByProjectDisplay {
    project: String,
    tests: usize,
    avg_ms: String,
    max_ms: String,
    avg_actions: String,
    avg_requests: String,
}

impl From<&ByProjectRow> for ByProjectDisplay {
    fn from(r: &ByProjectRow) -> Self {
        Self {
            project: r.project.clone(),
            tests: r.tests,
            avg_ms: ms(r.avg_ms),
            max_ms: ms(r.max_ms),
            avg_actions: format!("{:.2}", r.avg_actions),
            avg_requests: format!("{:.2}", r.avg_requests),
        }
    }
}

#[derive(Tabled)]
struct TraceTotalDisplay {
    total_ms: String,
    spans: usize,
    trace_id: String,
}

impl From<&TraceTotalRow> for TraceTotalDisplay {
    fn from(r: &TraceTotalRow) -> Self {
        Self {
            total_ms: ms(r.total_ms),
            spans: r.spans,
            trace_id: r.trace_id.clone(),
        }
    }
}

#[derive(Tabled)]
struct HotspotDisplay {
    max_ms: String,
    avg_ms: String,
    total_ms: String,
    count: usize,
    name: String,
}

impl From<&HotspotRow> for HotspotDisplay {
    fn from(r: &HotspotRow) -> Self {
        Self {
            max_ms: ms(r.max_ms),
            avg_ms: ms(r.avg_ms),
            total_ms: ms(r.total_ms),
            count: r.count,
            name: r.name.clone(),
        }
    }
}

#[derive(Tabled)]
struct TargetDisplay {
    max_ms: String,
    avg_ms: String,
    total_ms: String,
    count: usize,
    target: String,
}

impl From<&TargetRow> for TargetDisplay {
    fn from(r: &TargetRow) -> Self {
        Self {
            max_ms: ms(r.max_ms),
            avg_ms: ms(r.avg_ms),
            total_ms: ms(r.total_ms),
            count: r.count,
            target: r.target.clone(),
        }
    }
}

#[derive(Tabled)]
struct LongTaskProjectDisplay {
    project: String,
    tests: usize,
    tasks: usize,
    avg_ms_per_test: String,
    max_task_ms: String,
}

impl From<&LongTaskProjectRow> for LongTaskProjectDisplay {
    fn from(r: &LongTaskProjectRow) -> Self {
        Self {
            project: r.project.clone(),
            tests: r.tests,
            tasks: r.task_count,
            avg_ms_per_test: ms(r.avg_per_test_ms),
            max_task_ms: ms(r.max_ms),
        }
    }
}

#[derive(Tabled)]
struct AssetDisplay {
    max_ms: String,
    avg_ms: String,
    total_ms: String,
    count: usize,
    initiator: String,
    asset: String,
}

impl From<&AssetRow> for AssetDisplay {
    fn from(r: &AssetRow) -> Self {
        Self {
            max_ms: ms(r.max_ms),
            avg_ms: ms(r.avg_ms),
            total_ms: ms(r.total_ms),
            count: r.count,
            initiator: r.initiator.clone(),
            asset: r.name.clone(),
        }
    }
}

/// The full report text for `analysis`, with ranked tables bounded by `top`.
pub fn render(analysis: &Analysis, top: usize) -> String {
    if analysis.span_count == 0 {
        return "No spans found in the provided trace files.\n".to_string();
    }

    let mut out = String::new();
    if let Some(project) = &analysis.project_filter {
        out.push_str(&format!("Project filter: {project}\n\n"));
    }

    section::<SlowSpanDisplay>(
        &mut out,
        &format!("Top {top} slowest spans"),
        &top_display(&analysis.slowest_spans, top),
    );
    section::<E2eTestDisplay>(
        &mut out,
        &format!("Top {top} slowest e2e.test spans"),
        &top_display(&analysis.slowest_e2e_tests, top),
    );
    section::<HotspotDisplay>(
        &mut out,
        &format!("Top {top} e2e action hotspots (from e2e.action_top_json)"),
        &top_display(&analysis.action_hotspots, top),
    );
    section::<HotspotDisplay>(
        &mut out,
        &format!("Top {top} navigation phase hotspots (from e2e.navigation_top_json)"),
        &top_display(&analysis.navigation_phase_hotspots, top),
    );
    section::<TargetDisplay>(
        &mut out,
        &format!("Top {top} slow navigation targets"),
        &top_display(&analysis.navigation_targets, top),
    );
    section::<HotspotDisplay>(
        &mut out,
        &format!("Top {top} long-task hotspots (from e2e.long_tasks_json)"),
        &top_display(&analysis.long_task_hotspots, top),
    );
    section::<LongTaskProjectDisplay>(
        &mut out,
        "Long-task totals by project",
        &all_display(&analysis.long_task_by_project),
    );
    section::<HotspotDisplay>(
        &mut out,
        &format!("Top {top} resource initiator hotspots (from e2e.resource_summary_json)"),
        &top_display(&analysis.resource_initiators, top),
    );
    section::<AssetDisplay>(
        &mut out,
        &format!("Top {top} slow resource assets"),
        &top_display(&analysis.resource_assets, top),
    );
    section::<ByProjectDisplay>(
        &mut out,
        "E2E test duration by project",
        &all_display(&analysis.by_project),
    );
    section::<TraceTotalDisplay>(
        &mut out,
        "Trace totals (sum of span durations)",
        &top_display(&analysis.trace_totals, top),
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::analyze::{Analysis, SlowSpanRow};

    fn slow_row(duration_ms: f64) -> SlowSpanRow {
        SlowSpanRow {
            duration_ms,
            trace_id: "aa".into(),
            name: "e2e.test".into(),
            method: String::new(),
            uri: String::new(),
            busy_ns: String::new(),
            idle_ns: String::new(),
            source: "sample".into(),
        }
    }

    #[test]
    fn render_empty_is_no_spans_message() {
        let out = render(&Analysis::default(), 25);
        assert!(out.contains("No spans found in the provided trace files."));
    }

    #[test]
    fn render_project_filter_header() {
        let a = Analysis {
            span_count: 1,
            project_filter: Some("firefox".into()),
            slowest_spans: vec![slow_row(10.0)],
            ..Default::default()
        };
        assert!(render(&a, 25).starts_with("Project filter: firefox"));
    }

    #[test]
    fn render_slices_ranked_section() {
        let a = Analysis {
            span_count: 30,
            slowest_spans: (0..30).map(|i| slow_row(i as f64)).collect(),
            ..Default::default()
        };
        let out = render(&a, 5);
        // The slowest-spans table shows exactly `top` data rows: count the
        // formatted ms cells (each row renders one "N.000" duration).
        let data_rows = out.matches(".000").count();
        assert_eq!(data_rows, 5, "ranked table must be sliced to top");
    }

    #[test]
    fn render_emits_sections_in_canonical_order() {
        use crate::traces::analyze::analyze_spans;
        use crate::traces::parse::{parse_spans, Filters};
        const FIXTURE: &str = include_str!("testdata/otel-traces-sample.jsonl");
        let spans = parse_spans(FIXTURE, &Filters::default(), "sample").unwrap();
        let out = render(&analyze_spans(spans, None), 25);
        // Node main() (:1139-1150) fixes this section order.
        let order = [
            "Top 25 slowest spans",
            "slowest e2e.test spans",
            "e2e action hotspots",
            "navigation phase hotspots",
            "slow navigation targets",
            "long-task hotspots",
            "Long-task totals by project",
            "resource initiator hotspots",
            "slow resource assets",
            "E2E test duration by project",
            "Trace totals (sum of span durations)",
        ];
        let mut last = 0;
        for marker in order {
            let pos = out
                .find(marker)
                .unwrap_or_else(|| panic!("section missing from output: {marker}"));
            assert!(pos >= last, "section out of order: {marker}");
            last = pos;
        }
    }

    #[test]
    fn render_uses_tabled_not_manual_padding() {
        let a = Analysis {
            span_count: 1,
            slowest_spans: vec![slow_row(10.0)],
            ..Default::default()
        };
        let out = render(&a, 25);
        // tabled's sharp style draws box-drawing borders — proof we're not
        // hand-padding columns.
        assert!(out.contains('─'), "expected a tabled border glyph");
    }
}

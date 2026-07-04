//! Render an [`Analysis`](super::analyze::Analysis) to human-facing report tables.
//!
//! The display layer: it applies `--top` slicing and formats numbers, then lays
//! the rows out with `tabled` (no hand-rolled column padding — ADR-0028 cycle
//! decision). `analyze` already fully sorted every section; ranked tables are
//! sliced to `top` here, the all-row tables (added in later tasks) are not.

use tabled::settings::Style;
use tabled::{Table, Tabled};

use super::analyze::{Analysis, SlowSpanRow};

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

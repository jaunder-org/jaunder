//! Trace analysis: `Vec<Span>` → a typed [`Analysis`] of every report section.
//!
//! The reusable in-crate seam (ADR-0028 host analyzer). `analyze_spans` does the
//! whole computation with no I/O, so #33's `traces run` can call it in-process on
//! trace files it collected, and unit tests can drive it from a fixture without
//! spawning a process. Rows are **fully sorted** here; `--top` slicing is
//! [`super::render`]'s job. Port of the twelve `print*` functions in
//! `scripts/analyze-otel-traces`.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;

use super::parse::{
    Filters, Span, get_attr, interval_union_ms, parse_json_attr, read_spans, span_interval_ms,
    to_url_path,
};
use super::report::{AttemptKey, ReportedDurations};

/// Parse an `e2e.*` integer-count attribute (`0` when absent/non-numeric),
/// matching Node's `Number(getAttr(...) || "0")`.
fn count(raw: &Value, key: &str) -> u64 {
    get_attr(raw, key).parse().unwrap_or(0)
}

/// Read a finite JSON number; malformed and non-finite values are excluded.
fn field_f64(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// count / total / max accumulator shared by the hotspot sections.
#[derive(Default, Clone)]
struct Agg {
    count: usize,
    total_ms: f64,
    max_ms: f64,
}

impl Agg {
    fn add(&mut self, v: f64) {
        self.count += 1;
        self.total_ms += v;
        self.max_ms = self.max_ms.max(v);
    }
    fn avg(&self) -> f64 {
        self.total_ms / self.count as f64
    }
}

/// Turn name-keyed [`Agg`] groups into `HotspotRow`s sorted by `max_ms` desc.
fn hotspot_rows(groups: Vec<(String, Agg)>) -> Vec<HotspotRow> {
    let mut rows: Vec<HotspotRow> = groups
        .into_iter()
        .map(|(name, a)| HotspotRow {
            name,
            count: a.count,
            avg_ms: a.avg(),
            max_ms: a.max_ms,
            total_ms: a.total_ms,
        })
        .collect();
    sort_desc_by(&mut rows, |r| r.max_ms);
    rows
}

/// Only the `e2e.test` spans (the ones carrying the `e2e.*_json` blobs).
fn e2e_tests(spans: &[Span]) -> impl Iterator<Item = &Span> {
    spans.iter().filter(|s| s.name == "e2e.test")
}

/// The e2e project label a report groups on: the span's `e2e.project`, or `-`
/// when unset (Node's `getAttr(...) || "-"`).
fn project_label(project: &str) -> String {
    if project.is_empty() {
        "-".to_string()
    } else {
        project.to_string()
    }
}

/// The `e2e.test` name for a span, or `-` when unset (Node `getAttr(...) || "-"`).
fn e2e_test_name(s: &Span) -> String {
    let t = get_attr(&s.raw, "e2e.test");
    if t.is_empty() { "-".to_string() } else { t }
}

/// Every report section, as typed rows. Grown additively across the port; unbuilt
/// sections stay empty via `Default`.
#[derive(Debug, Default)]
pub struct Analysis {
    pub span_count: usize,
    pub project_filter: Option<String>,
    /// All spans, sorted by `duration_ms` descending (not sliced). Section 1.
    pub slowest_spans: Vec<SlowSpanRow>,
    /// `e2e.test` spans, slowest first. Section 2.
    pub slowest_e2e_tests: Vec<E2eTestRow>,
    /// `e2e.test` durations grouped by project, slowest average first. All rows
    /// (not sliced). Section 11.
    pub by_project: Vec<ByProjectRow>,
    /// Per-trace span-duration totals, largest first. Section 12.
    pub trace_totals: Vec<TraceTotalRow>,
    /// Action hotspots (`e2e.action_top_json`), `max_ms` desc. Section 3.
    pub action_hotspots: Vec<HotspotRow>,
    /// Boot-decomposition coverage per `(trace file, project)`. All rows. Section 5 (#818).
    pub boot_coverage: Vec<BootCoverageRow>,
    /// Navigation phase totals (`e2e.navigation_top_json`), `max_ms` desc. Section 4a.
    pub navigation_phase_hotspots: Vec<HotspotRow>,
    /// Slow navigation targets by URL path, `max_ms` desc. Section 4b.
    pub navigation_targets: Vec<TargetRow>,
    /// Long-task hotspots by task name (`e2e.long_tasks_json`), `max_ms` desc. Section 6a.
    pub long_task_hotspots: Vec<HotspotRow>,
    /// Long-task totals by project, `avg_per_test_ms` desc. All rows. Section 6b.
    pub long_task_by_project: Vec<LongTaskProjectRow>,
    /// Resource initiator hotspots (`e2e.resource_summary_json`), `max_ms` desc. Section 7a.
    pub resource_initiators: Vec<HotspotRow>,
    /// Slow resource assets, `max_ms` desc. Section 7b.
    pub resource_assets: Vec<AssetRow>,
    /// Per-test span coverage, largest uncovered remainder first. Section 13 (#794).
    pub span_coverage: Vec<SpanCoverageRow>,
    /// Why the coverage section is empty, when it is. `None` means it has rows.
    /// Never silently absent: an empty section and an unsupplied report look
    /// identical otherwise, and one of those is a broken build.
    pub span_coverage_note: Option<String>,
}

/// The `e2e.test.lifecycle` envelope introduced in #794. Its children are the
/// named phases; the union of those is what "covered" means.
pub const LIFECYCLE_SPAN_NAME: &str = "e2e.test.lifecycle";

/// One row of "per-test span coverage" (#794, AC-4).
///
/// `covered_ms` is the interval **union** of the lifecycle envelope's children —
/// how much of the attempt's wall-clock lands inside a *named* phase. Comparing
/// that against Playwright's own duration is what makes the previously-invisible
/// fixture overhead visible as a number rather than an inference.
#[derive(Debug, Clone)]
pub struct SpanCoverageRow {
    pub project: String,
    pub test: String,
    pub reported_ms: f64,
    pub covered_ms: f64,
    pub uncovered_ms: f64,
    pub uncovered_pct: f64,
}

/// One row of the "slowest spans" table.
#[derive(Debug, Clone)]
pub struct SlowSpanRow {
    pub duration_ms: f64,
    pub trace_id: String,
    pub name: String,
    pub method: String,
    pub uri: String,
    pub busy_ns: String,
    pub idle_ns: String,
    pub source: String,
}

/// One row of "slowest e2e.test spans".
#[derive(Debug, Clone)]
pub struct E2eTestRow {
    pub duration_ms: f64,
    pub project: String,
    pub actions: u64,
    pub requests: u64,
    pub trace_id: String,
    pub test: String,
}

/// One row of "E2E test duration by project".
#[derive(Debug, Clone)]
pub struct ByProjectRow {
    pub project: String,
    pub tests: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub avg_actions: f64,
    pub avg_requests: f64,
}

/// One row of "Boot decomposition coverage" (#818, spec D3) — how much of the boot
/// decomposition a single `(trace file, project)` population actually captured.
#[derive(Debug, Clone)]
pub struct BootCoverageRow {
    /// The trace file this came from. **Load-bearing:** `projectName` is the browser
    /// and names no backend (`traces/run.rs:99-101`), so keying on `project` alone
    /// pools sqlite with postgres into one row.
    pub source: String,
    pub project: String,
    pub navigations: u64,
    pub mounted: u64,
    pub full_marks: u64,
    pub dropped: u64,
}

/// One row of "Trace totals".
#[derive(Debug, Clone)]
pub struct TraceTotalRow {
    pub trace_id: String,
    pub total_ms: f64,
    pub spans: usize,
}

/// A generic name-keyed hotspot row (action / navigation-phase / long-task /
/// resource-initiator sections all share this shape).
#[derive(Debug, Clone)]
pub struct HotspotRow {
    pub name: String,
    pub count: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub total_ms: f64,
}

/// A slow-navigation-target row (keyed by URL path, Node navigation `urlTotals`).
#[derive(Debug, Clone)]
pub struct TargetRow {
    pub target: String,
    pub count: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub total_ms: f64,
}

/// Long-task totals by project (Node `printE2eLongTaskHotspots` `projectRows`).
#[derive(Debug, Clone)]
pub struct LongTaskProjectRow {
    pub project: String,
    pub tests: usize,
    pub task_count: usize,
    pub avg_per_test_ms: f64,
    pub max_ms: f64,
}

/// A slow-resource-asset row (Node resource `assetTotals`).
#[derive(Debug, Clone)]
pub struct AssetRow {
    pub name: String,
    pub initiator: String,
    pub count: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub total_ms: f64,
}

/// Sort a `f64`-keyed vector descending, treating the key as a total order (NaN
/// sinks to the end). Used by every ranked section.
fn sort_desc_by<T>(rows: &mut [T], key: impl Fn(&T) -> f64) {
    rows.sort_by(|a, b| {
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Look up (or first-insert) the accumulator for `key` in an insertion-ordered
/// `Vec` of `(key, acc)`. First-seen order mirrors the JS `Map` the Node script
/// groups with, so tie-order in the sorted output matches. Group counts are tiny
/// (projects, traces), so linear search is fine.
fn entry<'a, V>(groups: &'a mut Vec<(String, V)>, key: &str, init: impl Fn() -> V) -> &'a mut V {
    if let Some(idx) = groups.iter().position(|(k, _)| k == key) {
        &mut groups[idx].1
    } else {
        groups.push((key.to_string(), init()));
        &mut groups.last_mut().unwrap().1
    }
}

/// Section 3 — action hotspots from `e2e.action_top_json`. No `< 0` guard (Node
/// only checks `isFinite`); empty names are skipped.
fn action_hotspot_rows(spans: &[Span]) -> Result<Vec<HotspotRow>> {
    let mut groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let actions = parse_json_attr(&s.raw, "e2e.action_top_json", &s.source)?;
        let Some(arr) = actions.as_ref().and_then(Value::as_array) else {
            continue;
        };
        for action in arr {
            let name = action.get("name").and_then(Value::as_str).unwrap_or("");
            let Some(dur) = field_f64(action, "durationMs") else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            entry(&mut groups, name, Agg::default).add(dur);
        }
    }
    Ok(hotspot_rows(groups))
}

/// The navigation phase fields aggregated as `navigation.<label>` (Node `addPhase`
/// set, :373-387).
const NAV_PHASES: [(&str, &str); 5] = [
    ("navigation.total", "totalMs"),
    ("navigation.request", "requestMs"),
    (
        "navigation.commit_to_domcontentloaded",
        "commitToDomContentLoadedMs",
    ),
    ("navigation.commit_to_mount", "commitToMountMs"),
    (
        "navigation.domcontentloaded_to_load",
        "domContentLoadedToLoadMs",
    ),
];

/// Section 4 — navigation phase totals + slow navigation targets from
/// `e2e.navigation_top_json`. Phases and targets drop negative/non-finite values.
fn navigation_sections(spans: &[Span]) -> Result<(Vec<HotspotRow>, Vec<TargetRow>)> {
    let mut phase_groups: Vec<(String, Agg)> = Vec::new();
    let mut url_groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let navs = parse_json_attr(&s.raw, "e2e.navigation_top_json", &s.source)?;
        let Some(arr) = navs.as_ref().and_then(Value::as_array) else {
            continue;
        };
        for nav in arr {
            for (label, field) in NAV_PHASES {
                if let Some(v) = field_f64(nav, field)
                    && v >= 0.0
                {
                    entry(&mut phase_groups, label, Agg::default).add(v);
                }
            }
            let path = to_url_path(nav.get("url").and_then(Value::as_str).unwrap_or(""));
            if path.is_empty() {
                continue;
            }
            if let Some(total) = field_f64(nav, "totalMs")
                && total >= 0.0
            {
                entry(&mut url_groups, &path, Agg::default).add(total);
            }
        }
    }
    let phase_rows = hotspot_rows(phase_groups);
    let mut target_rows: Vec<TargetRow> = url_groups
        .into_iter()
        .map(|(target, a)| TargetRow {
            target,
            count: a.count,
            avg_ms: a.avg(),
            max_ms: a.max_ms,
            total_ms: a.total_ms,
        })
        .collect();
    sort_desc_by(&mut target_rows, |r| r.max_ms);
    Ok((phase_rows, target_rows))
}

/// The fewest `bootPhases` entries a fully decomposed navigation carries.
/// `bootPhasesFrom` (`end2end/tests/fixtures.ts`) emits one phase per adjacent mark
/// pair, so today's four `jaunder.*` marks yield three. **A floor, never an
/// equality:** `client::perf` may gain a mark, and pinning the count would report
/// that as a total coverage blackout — exactly the silent failure #818 exists to
/// eliminate.
const MIN_BOOT_PHASES: usize = 3;

/// Section 5 — boot-decomposition coverage from `e2e.navigation_top_json` (#818).
///
/// Grouped by `(source, project)`, not by project: `projectName` is the browser and
/// names no backend (`traces/run.rs:99-101`), so on `project` alone a sqlite run and
/// a postgres run of the same browser pool into a single meaningless row. A
/// `(source, project)` that produced no navigations still gets a zeroed row, so a
/// combo that captured nothing is visible rather than absent.
///
/// `mounted` is **proxied** by a non-null `commitToMountMs` — an approximation, not
/// an equivalence. That field is non-null iff *both* `committedMs` and `mountedMs`
/// were set (`end2end/tests/fixtures.ts:618-621`), so a navigation that mounted but
/// whose commit was never observed (the `state.pending.shift()` path in
/// `capture-trace.ts`) drops out of both the numerator and the denominator.
///
/// `dropped` sums `e2e.navigation_top_dropped`, because `e2e.navigation_top_json`
/// is the top 20 *by duration* per test — a biased sample, not a census. Without it
/// a truncated capture reads as complete coverage.
fn boot_coverage_rows(spans: &[Span]) -> Result<Vec<BootCoverageRow>> {
    let mut rows: Vec<BootCoverageRow> = Vec::new();
    for s in e2e_tests(spans) {
        let project = project_label(&s.project);
        let existing = rows
            .iter()
            .position(|r| r.source == s.source && r.project == project);
        let idx = match existing {
            Some(idx) => idx,
            None => {
                rows.push(BootCoverageRow {
                    source: s.source.clone(),
                    project,
                    navigations: 0,
                    mounted: 0,
                    full_marks: 0,
                    dropped: 0,
                });
                rows.len() - 1
            }
        };
        let row = &mut rows[idx];
        row.dropped += count(&s.raw, "e2e.navigation_top_dropped");
        let navs = parse_json_attr(&s.raw, "e2e.navigation_top_json", &s.source)?;
        let Some(arr) = navs.as_ref().and_then(Value::as_array) else {
            continue;
        };
        for nav in arr {
            row.navigations += 1;
            if field_f64(nav, "commitToMountMs").is_some() {
                row.mounted += 1;
            }
            let phases = nav
                .get("bootPhases")
                .and_then(Value::as_object)
                .map_or(0, |phases| phases.len());
            let current =
                nav.get("wasmTimingSchema").and_then(Value::as_str) == Some("direct-init-v1");
            if current
                && phases >= MIN_BOOT_PHASES
                && field_f64(nav, "wasmInitStartMs").is_some()
                && field_f64(nav, "wasmInitStartToBootEntryMs").is_some()
            {
                row.full_marks += 1;
            }
        }
    }
    Ok(rows)
}

/// Section 6 — long-task hotspots by task name + per-project totals from
/// `e2e.long_tasks_json`. Negative/non-finite durations are dropped.
fn long_task_sections(spans: &[Span]) -> Result<(Vec<HotspotRow>, Vec<LongTaskProjectRow>)> {
    #[derive(Default)]
    struct ProjAgg {
        tests: usize,
        task_count: usize,
        total_ms: f64,
        max_ms: f64,
    }
    let mut name_groups: Vec<(String, Agg)> = Vec::new();
    let mut proj_groups: Vec<(String, ProjAgg)> = Vec::new();
    for s in e2e_tests(spans) {
        let tasks = parse_json_attr(&s.raw, "e2e.long_tasks_json", &s.source)?;
        let Some(arr) = tasks.as_ref().and_then(Value::as_array) else {
            continue;
        };
        let pa = entry(
            &mut proj_groups,
            &project_label(&s.project),
            ProjAgg::default,
        );
        pa.tests += 1;
        for task in arr {
            let Some(dur) = field_f64(task, "duration") else {
                continue;
            };
            if dur < 0.0 {
                continue;
            }
            let name = task
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("longtask");
            entry(&mut name_groups, name, Agg::default).add(dur);
            pa.task_count += 1;
            pa.total_ms += dur;
            pa.max_ms = pa.max_ms.max(dur);
        }
    }
    let hotspots = hotspot_rows(name_groups);
    let mut project_rows: Vec<LongTaskProjectRow> = proj_groups
        .into_iter()
        .map(|(project, p)| LongTaskProjectRow {
            project,
            tests: p.tests,
            task_count: p.task_count,
            avg_per_test_ms: if p.tests > 0 {
                p.total_ms / p.tests as f64
            } else {
                0.0
            },
            max_ms: p.max_ms,
        })
        .collect();
    sort_desc_by(&mut project_rows, |r| r.avg_per_test_ms);
    Ok((hotspots, project_rows))
}

/// Section 7 — resource initiator hotspots + slow assets from
/// `e2e.resource_summary_json.topSlow`. Negative/non-finite durations dropped.
fn resource_sections(spans: &[Span]) -> Result<(Vec<HotspotRow>, Vec<AssetRow>)> {
    struct AssetAgg {
        initiator: String,
        agg: Agg,
    }
    let mut init_groups: Vec<(String, Agg)> = Vec::new();
    let mut asset_groups: Vec<(String, AssetAgg)> = Vec::new();
    for s in e2e_tests(spans) {
        let summary = parse_json_attr(&s.raw, "e2e.resource_summary_json", &s.source)?;
        let Some(items) = summary
            .as_ref()
            .and_then(|value| value.get("topSlow"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for item in items {
            let Some(dur) = field_f64(item, "durationMs") else {
                continue;
            };
            if dur < 0.0 {
                continue;
            }
            let initiator = item
                .get("initiatorType")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown")
                .to_string();
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(to_url_path)
                .unwrap_or_else(|| "unknown".to_string());
            entry(&mut init_groups, &initiator, Agg::default).add(dur);
            if let Some(idx) = asset_groups.iter().position(|(k, _)| *k == name) {
                asset_groups[idx].1.agg.add(dur);
            } else {
                let mut agg = Agg::default();
                agg.add(dur);
                asset_groups.push((name, AssetAgg { initiator, agg }));
            }
        }
    }
    let initiator_rows = hotspot_rows(init_groups);
    let mut asset_rows: Vec<AssetRow> = asset_groups
        .into_iter()
        .map(|(name, a)| AssetRow {
            name,
            initiator: a.initiator,
            count: a.agg.count,
            avg_ms: a.agg.avg(),
            max_ms: a.agg.max_ms,
            total_ms: a.agg.total_ms,
        })
        .collect();
    sort_desc_by(&mut asset_rows, |r| r.max_ms);
    Ok((initiator_rows, asset_rows))
}

/// Per-test span coverage: how much of each attempt's wall-clock lands inside a
/// named phase of the lifecycle tree (#794, AC-4).
///
/// Covered time is the interval **union of the envelope's children**, not the
/// envelope's own duration. The envelope spans the whole lifecycle by
/// construction, so measuring it against Playwright's duration would report ~100 %
/// coverage no matter how much time sat between the phases. The union of the named
/// phases is the honest numerator.
///
/// A test with no matching report entry is skipped rather than rendered with a
/// zero denominator — see `report::ReportedDurations`.
pub fn span_coverage(spans: &[Span], reported: &ReportedDurations) -> Vec<SpanCoverageRow> {
    let mut children: HashMap<&str, Vec<&Span>> = HashMap::new();
    for span in spans {
        if span.parent_span_id.is_empty() {
            continue;
        }
        children
            .entry(span.parent_span_id.as_str())
            .or_default()
            .push(span);
    }

    let mut rows: Vec<SpanCoverageRow> = spans
        .iter()
        .filter(|span| span.name == LIFECYCLE_SPAN_NAME)
        .filter_map(|envelope| {
            let key = AttemptKey {
                test: get_attr(&envelope.raw, "e2e.test"),
                project: get_attr(&envelope.raw, "e2e.project"),
                retry: get_attr(&envelope.raw, "e2e.retry")
                    .parse::<u64>()
                    .unwrap_or(0),
            };
            // Scoped by the envelope's own trace file: sqlite and postgres produce
            // identical (test, project, retry) keys with different durations.
            let reported_ms = reported.get(&envelope.source, &key)?;
            let intervals: Vec<(f64, f64)> = children
                .get(envelope.span_id.as_str())
                .map(|kids| {
                    kids.iter()
                        .filter_map(|kid| span_interval_ms(&kid.raw))
                        .collect()
                })
                .unwrap_or_default();
            let covered_ms = interval_union_ms(intervals);
            // Clamped: clock skew between the Node-side span stamps and
            // Playwright's own timing can put covered marginally above reported,
            // and a negative "uncovered" would read as nonsense.
            let uncovered_ms = (reported_ms - covered_ms).max(0.0);
            Some(SpanCoverageRow {
                project: project_label(&key.project),
                test: key.test,
                reported_ms,
                covered_ms,
                uncovered_ms,
                uncovered_pct: if reported_ms > 0.0 {
                    uncovered_ms / reported_ms * 100.0
                } else {
                    0.0
                },
            })
        })
        .collect();
    sort_desc_by(&mut rows, |row| row.uncovered_ms);
    rows
}

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
    let coverage = span_coverage(&spans, reported);
    let coverage_note = coverage_note(&spans, reported, &coverage);
    let mut analysis = analyze_spans_inner(spans, project_filter)?;
    analysis.span_coverage = coverage;
    analysis.span_coverage_note = coverage_note;
    Ok(analysis)
}

/// Explain an empty coverage section, so "no report supplied" is never mistaken
/// for "everything is attributed".
fn coverage_note(
    spans: &[Span],
    reported: &ReportedDurations,
    coverage: &[SpanCoverageRow],
) -> Option<String> {
    if !coverage.is_empty() {
        return None;
    }
    if reported.is_empty() {
        return Some(
            "no --playwright-report supplied; per-test coverage needs Playwright's \
             own durations as the denominator"
                .to_owned(),
        );
    }
    if !spans.iter().any(|s| s.name == LIFECYCLE_SPAN_NAME) {
        return Some(format!(
            "no `{LIFECYCLE_SPAN_NAME}` spans in the capture (pre-#794 traces have none)"
        ));
    }
    Some(format!(
        "no lifecycle span matched a report entry ({} attempt(s) in the report)",
        reported.len()
    ))
}

fn analyze_spans_inner(spans: Vec<Span>, project_filter: Option<String>) -> Result<Analysis> {
    let mut slowest_spans: Vec<SlowSpanRow> = spans
        .iter()
        .map(|s| SlowSpanRow {
            duration_ms: s.duration_ms,
            trace_id: s.trace_id.clone(),
            name: s.name.clone(),
            method: s.method.clone(),
            uri: s.uri.clone(),
            busy_ns: s.busy_ns.clone(),
            idle_ns: s.idle_ns.clone(),
            source: s.source.clone(),
        })
        .collect();
    sort_desc_by(&mut slowest_spans, |r| r.duration_ms);

    // Section 2 — slowest e2e.test spans.
    let mut slowest_e2e_tests: Vec<E2eTestRow> = spans
        .iter()
        .filter(|s| s.name == "e2e.test")
        .map(|s| E2eTestRow {
            duration_ms: s.duration_ms,
            project: project_label(&s.project),
            actions: count(&s.raw, "e2e.action_count"),
            requests: count(&s.raw, "e2e.request_count"),
            trace_id: s.trace_id.clone(),
            test: e2e_test_name(s),
        })
        .collect();
    sort_desc_by(&mut slowest_e2e_tests, |r| r.duration_ms);

    // Section 11 — e2e.test duration grouped by project.
    #[derive(Default)]
    struct ProjAgg {
        tests: usize,
        total_ms: f64,
        max_ms: f64,
        actions: u64,
        requests: u64,
    }
    let mut proj_groups: Vec<(String, ProjAgg)> = Vec::new();
    for s in spans.iter().filter(|s| s.name == "e2e.test") {
        let a = entry(
            &mut proj_groups,
            &project_label(&s.project),
            ProjAgg::default,
        );
        a.tests += 1;
        a.total_ms += s.duration_ms;
        a.max_ms = a.max_ms.max(s.duration_ms);
        a.actions += count(&s.raw, "e2e.action_count");
        a.requests += count(&s.raw, "e2e.request_count");
    }
    let mut by_project: Vec<ByProjectRow> = proj_groups
        .into_iter()
        .map(|(project, a)| ByProjectRow {
            project,
            tests: a.tests,
            avg_ms: a.total_ms / a.tests as f64,
            max_ms: a.max_ms,
            avg_actions: a.actions as f64 / a.tests as f64,
            avg_requests: a.requests as f64 / a.tests as f64,
        })
        .collect();
    sort_desc_by(&mut by_project, |r| r.avg_ms);

    // Section 12 — per-trace duration totals (all spans).
    #[derive(Default)]
    struct TraceAgg {
        total_ms: f64,
        spans: usize,
    }
    let mut trace_groups: Vec<(String, TraceAgg)> = Vec::new();
    for s in &spans {
        let a = entry(&mut trace_groups, &s.trace_id, TraceAgg::default);
        a.total_ms += s.duration_ms;
        a.spans += 1;
    }
    let mut trace_totals: Vec<TraceTotalRow> = trace_groups
        .into_iter()
        .map(|(trace_id, a)| TraceTotalRow {
            trace_id,
            total_ms: a.total_ms,
            spans: a.spans,
        })
        .collect();
    sort_desc_by(&mut trace_totals, |r| r.total_ms);

    // Sections 3, 4, 6, 7 — the JSON-attribute hotspots.
    let action_hotspots = action_hotspot_rows(&spans)?;
    let (navigation_phase_hotspots, navigation_targets) = navigation_sections(&spans)?;
    let boot_coverage = boot_coverage_rows(&spans)?;
    let (long_task_hotspots, long_task_by_project) = long_task_sections(&spans)?;
    let (resource_initiators, resource_assets) = resource_sections(&spans)?;

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
        // Filled in by `analyze_spans`, which owns the report join.
        span_coverage: Vec::new(),
        span_coverage_note: None,
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
    analyze_spans(spans, filters.project, reported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::parse::parse_spans;

    const FIXTURE: &str = include_str!("testdata/otel-traces-sample.jsonl");

    fn fixture_spans() -> Vec<Span> {
        let mut spans = parse_spans(FIXTURE, &Filters::default(), "sample").unwrap();
        for span in &mut spans {
            if span.project == "chromium"
                && let Some(attributes) = span.raw["attributes"].as_array_mut()
            {
                attributes.retain(|attribute| {
                    attribute["key"].as_str() != Some("e2e.navigation_top_json")
                });
            }
        }
        spans
    }

    // --- span coverage (#794, AC-4) -----------------------------------------
    //
    // Built inline rather than added to the shared fixture: several tests above
    // assert exact span counts against it, so growing it would redden them for
    // reasons unrelated to what they check.

    fn attr(key: &str, value: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "key": key, "value": value })
    }

    /// One span at `[start_ms, end_ms)`, in the shape `parse_spans` reads.
    fn timed_span(
        name: &str,
        span_id: &str,
        parent: &str,
        start_ms: u64,
        end_ms: u64,
        extra: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        let mut attributes = vec![attr(
            "e2e.project",
            serde_json::json!({ "stringValue": "chromium" }),
        )];
        attributes.extend(extra);
        serde_json::json!({
            "name": name,
            "spanId": span_id,
            "parentSpanId": parent,
            "startTimeUnixNano": (start_ms * 1_000_000).to_string(),
            "endTimeUnixNano": (end_ms * 1_000_000).to_string(),
            "attributes": attributes,
        })
    }

    /// A lifecycle envelope with two children that OVERLAP, plus the identity
    /// attributes the report join needs.
    fn lifecycle_tree() -> Vec<Span> {
        let identity = || {
            vec![
                attr("e2e.test", serde_json::json!({ "stringValue": "logs in" })),
                attr("e2e.retry", serde_json::json!({ "intValue": "0" })),
            ]
        };
        let line = serde_json::json!({
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [
                        timed_span("e2e.test.lifecycle", "aa", "", 1_000, 1_500, identity()),
                        // 1000-1200 and 1150-1300 overlap by 50ms: union is 300,
                        // a naive sum would say 350.
                        timed_span("e2e.context_mint", "bb", "aa", 1_000, 1_200, vec![]),
                        timed_span("e2e.test", "cc", "aa", 1_150, 1_300, vec![]),
                    ]
                }]
            }]
        });
        parse_spans(&line.to_string(), &Filters::default(), "coverage").unwrap()
    }

    fn reported(ms: f64) -> ReportedDurations {
        ReportedDurations::from_value(&serde_json::json!({
            "suites": [{
                "specs": [{
                    "title": "logs in",
                    "tests": [{
                        "projectName": "chromium",
                        "results": [{ "retry": 0, "duration": ms }]
                    }]
                }]
            }]
        }))
    }

    #[test]
    fn span_coverage_unions_overlapping_children() {
        let rows = span_coverage(&lifecycle_tree(), &reported(500.0));
        assert_eq!(rows.len(), 1);
        // The point of the section: overlapping phases are counted once.
        assert_eq!(rows[0].covered_ms, 300.0);
        assert_eq!(rows[0].reported_ms, 500.0);
        assert_eq!(rows[0].uncovered_ms, 200.0);
        assert!((rows[0].uncovered_pct - 40.0).abs() < 1e-9);
    }

    #[test]
    fn span_coverage_measures_children_not_the_envelope() {
        // The envelope spans 1000-1500 by construction, so measuring IT would
        // report full coverage however much time sat between the phases.
        let rows = span_coverage(&lifecycle_tree(), &reported(500.0));
        assert!(
            rows[0].covered_ms < 500.0,
            "covered must come from the named phases, not the envelope",
        );
    }

    #[test]
    fn span_coverage_clamps_a_negative_remainder() {
        // Clock skew can put covered marginally above reported; a negative
        // "uncovered" would render as nonsense.
        let rows = span_coverage(&lifecycle_tree(), &reported(100.0));
        assert_eq!(rows[0].uncovered_ms, 0.0);
    }

    #[test]
    fn span_coverage_skips_a_test_absent_from_the_report() {
        let rows = span_coverage(&lifecycle_tree(), &ReportedDurations::default());
        assert!(
            rows.is_empty(),
            "no denominator means no row — never a zero-denominator row",
        );
    }

    #[test]
    fn coverage_note_distinguishes_no_report_from_no_lifecycle_spans() {
        // An empty section and a missing report must not look alike.
        let no_report =
            analyze_spans(lifecycle_tree(), None, &ReportedDurations::default()).unwrap();
        assert!(
            no_report
                .span_coverage_note
                .as_deref()
                .unwrap()
                .contains("playwright-report")
        );

        let no_lifecycle = analyze_spans(fixture_spans(), None, &reported(500.0)).unwrap();
        assert!(
            no_lifecycle
                .span_coverage_note
                .as_deref()
                .unwrap()
                .contains("lifecycle")
        );
    }

    #[test]
    fn coverage_note_is_absent_when_the_section_has_rows() {
        let analysis = analyze_spans(lifecycle_tree(), None, &reported(500.0)).unwrap();
        assert!(analysis.span_coverage_note.is_none());
        assert_eq!(analysis.span_coverage.len(), 1);
    }

    #[test]
    fn retry_attempts_join_separately() {
        // Spans are exported per attempt; joining a retry's spans against attempt
        // 0's wall-clock would silently mis-state coverage.
        let rows = span_coverage(&lifecycle_tree(), &reported(500.0));
        assert_eq!(rows.len(), 1, "retry 0 matched exactly one report entry");

        let only_retry_one = ReportedDurations::from_value(&serde_json::json!({
            "suites": [{
                "specs": [{
                    "title": "logs in",
                    "tests": [{
                        "projectName": "chromium",
                        "results": [{ "retry": 1, "duration": 500.0 }]
                    }]
                }]
            }]
        }));
        assert!(
            span_coverage(&lifecycle_tree(), &only_retry_one).is_empty(),
            "a retry-0 span tree must not match a retry-1 report entry",
        );
    }

    // --- boot-decomposition coverage (#818) ---------------------------------

    /// One navigation entry of `e2e.navigation_top_json`, carrying only the
    /// current schema fields the coverage section reads.
    fn nav(
        commit_to_mount_ms: Option<f64>,
        phases: Option<usize>,
        wasm_init_start_ms: Option<f64>,
    ) -> serde_json::Value {
        let boot_phases = match phases {
            None => serde_json::Value::Null,
            Some(n) => serde_json::Value::Object(
                (0..n)
                    .map(|i| (format!("a{i}->a{}", i + 1), serde_json::json!(1.0)))
                    .collect(),
            ),
        };
        serde_json::json!({
            "wasmTimingSchema": "direct-init-v1",
            "commitToMountMs": commit_to_mount_ms,
            "bootPhases": boot_phases,
            "wasmInitStartMs": wasm_init_start_ms,
            "wasmInitStartToBootEntryMs": wasm_init_start_ms,
        })
    }

    /// A fully decomposed, mounted navigation (the post-fix steady state).
    fn full_nav() -> serde_json::Value {
        nav(Some(120.0), Some(3), Some(40.0))
    }

    /// One `e2e.test` span carrying `navs` as `e2e.navigation_top_json`, parsed out
    /// of the trace file `source` — the identity `boot_coverage_rows` keys on
    /// alongside the project.
    fn boot_span(
        source: &str,
        project: &str,
        dropped: u64,
        navs: Vec<serde_json::Value>,
    ) -> Vec<Span> {
        let line = serde_json::json!({
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [{
                        "name": "e2e.test",
                        "attributes": [
                            attr("e2e.project", serde_json::json!({ "stringValue": project })),
                            attr(
                                "e2e.navigation_top_json",
                                serde_json::json!({
                                    "stringValue": serde_json::Value::Array(navs).to_string()
                                }),
                            ),
                            attr(
                                "e2e.navigation_top_dropped",
                                serde_json::json!({ "intValue": dropped.to_string() }),
                            ),
                        ],
                    }]
                }]
            }]
        });
        parse_spans(&line.to_string(), &Filters::default(), source).unwrap()
    }

    fn row_for<'a>(rows: &'a [BootCoverageRow], project: &str) -> &'a BootCoverageRow {
        rows.iter()
            .find(|r| r.project == project)
            .unwrap_or_else(|| panic!("no coverage row for {project}"))
    }

    #[test]
    fn boot_coverage_counts_mounted_and_fully_marked_navigations_per_project() {
        // firefox: 2 navigations, both mounted, neither decomposed — the #818 blackout.
        // chromium: 2 navigations, 1 mounted and fully marked, 1 never mounted.
        let mut spans = boot_span(
            "sqlite",
            "firefox",
            0,
            vec![nav(Some(300.0), None, None), nav(Some(280.0), None, None)],
        );
        spans.extend(boot_span(
            "sqlite",
            "chromium",
            0,
            vec![full_nav(), nav(None, None, None)],
        ));

        let rows = boot_coverage_rows(&spans).unwrap();
        let ff = row_for(&rows, "firefox");
        assert_eq!((ff.navigations, ff.mounted, ff.full_marks), (2, 2, 0));
        let chr = row_for(&rows, "chromium");
        assert_eq!((chr.navigations, chr.mounted, chr.full_marks), (2, 1, 1));
    }

    #[test]
    fn boot_coverage_separates_rows_by_source_file() {
        // `projectName` is the browser and names no backend (`traces/run.rs:99-101`),
        // so keying on project alone would pool sqlite navigations with postgres ones
        // into a single, meaningless row.
        let mut spans = boot_span("sqlite", "firefox", 0, vec![full_nav()]);
        spans.extend(boot_span("postgres", "firefox", 0, vec![full_nav()]));

        let rows = boot_coverage_rows(&spans).unwrap();
        assert_eq!(rows.len(), 2, "sqlite and postgres must not be pooled");
        assert!(rows.iter().all(|r| r.project == "firefox"));
        assert!(rows.iter().any(|r| r.source == "sqlite"));
        assert!(rows.iter().any(|r| r.source == "postgres"));
        assert!(rows.iter().all(|r| r.navigations == 1));
    }

    #[test]
    fn a_navigation_is_mounted_iff_commit_to_mount_is_present() {
        // `commitToMountMs` is non-null iff `committedMs` AND `mountedMs` were both
        // set (`end2end/tests/fixtures.ts:618-621`), so it is the mounted proxy.
        let spans = boot_span("sqlite", "firefox", 0, vec![nav(None, Some(3), Some(40.0))]);
        let rows = boot_coverage_rows(&spans).unwrap();
        assert_eq!(rows[0].navigations, 1);
        assert_eq!(
            rows[0].mounted, 0,
            "boot phases alone must not count as mounted",
        );
    }

    #[test]
    fn full_marks_accepts_extra_boot_phases_but_not_missing_ones() {
        // A FOURTH phase means `client::perf` gained a mark. Requiring exactly three
        // would read that as a total coverage blackout — the very class of silent
        // failure #818 exists to eliminate.
        let mut spans = boot_span(
            "sqlite",
            "firefox",
            0,
            vec![nav(Some(1.0), Some(4), Some(40.0))],
        );
        spans.extend(boot_span(
            "sqlite",
            "chromium",
            0,
            vec![nav(Some(1.0), Some(2), Some(40.0))],
        ));
        // A current navigation missing either exclusive pre-boot segment is not
        // a full mark set.
        spans.extend(boot_span(
            "sqlite",
            "webkit",
            0,
            vec![nav(Some(1.0), Some(3), None)],
        ));

        let rows = boot_coverage_rows(&spans).unwrap();
        assert_eq!(row_for(&rows, "firefox").full_marks, 1, "4 phases is full");
        assert_eq!(
            row_for(&rows, "chromium").full_marks,
            0,
            "2 phases is short",
        );
        assert_eq!(
            row_for(&rows, "webkit").full_marks,
            0,
            "missing exclusive fields is short",
        );
    }

    #[test]
    fn boot_coverage_sums_navigation_top_dropped_so_truncation_is_never_silent() {
        // `e2e.navigation_top_json` is the top 20 BY DURATION per test — a biased
        // sample, not a census. Without the dropped count a truncated capture reads
        // as complete coverage.
        let mut spans = boot_span("sqlite", "firefox", 3, vec![full_nav()]);
        spans.extend(boot_span("sqlite", "firefox", 5, vec![full_nav()]));

        let rows = boot_coverage_rows(&spans).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].navigations, 2);
        assert_eq!(rows[0].dropped, 8, "dropped accumulates across tests");
    }

    #[test]
    fn slowest_spans_sorted_desc_and_complete() {
        let spans = fixture_spans();
        let n = spans.len();
        assert!(n > 0, "fixture must have spans");
        let a = analyze_spans(spans, None, &ReportedDurations::default()).unwrap();
        assert_eq!(a.span_count, n);
        // Every span present (not sliced), sorted by duration descending.
        assert_eq!(a.slowest_spans.len(), n);
        for pair in a.slowest_spans.windows(2) {
            assert!(
                pair[0].duration_ms >= pair[1].duration_ms,
                "not sorted desc"
            );
        }
    }

    #[test]
    fn slowest_e2e_tests_only_e2e_test_spans() {
        let a = analyze_spans(fixture_spans(), None, &ReportedDurations::default()).unwrap();
        // Two e2e.test spans in the fixture; the HTTP spans are excluded.
        assert_eq!(a.slowest_e2e_tests.len(), 2);
        // Slowest first: firefox (5000ms) then chromium (3000ms).
        let first = &a.slowest_e2e_tests[0];
        assert_eq!(first.project, "firefox");
        assert_eq!(first.duration_ms, 5000.0);
        assert_eq!(first.actions, 40);
        assert_eq!(first.requests, 12);
        assert_eq!(first.test, "timeline heavy");
        assert_eq!(a.slowest_e2e_tests[1].project, "chromium");
    }

    #[test]
    fn by_project_groups_and_averages() {
        let a = analyze_spans(fixture_spans(), None, &ReportedDurations::default()).unwrap();
        // One row per project, each with a single test; sorted by avg_ms desc.
        assert_eq!(a.by_project.len(), 2);
        let ff = &a.by_project[0];
        assert_eq!(ff.project, "firefox");
        assert_eq!(ff.tests, 1);
        assert_eq!(ff.avg_ms, 5000.0);
        assert_eq!(ff.max_ms, 5000.0);
        assert_eq!(ff.avg_actions, 40.0);
        assert_eq!(ff.avg_requests, 12.0);
        assert_eq!(a.by_project[1].project, "chromium");
        assert_eq!(a.by_project[1].avg_ms, 3000.0);
    }

    #[test]
    fn trace_totals_sum_per_trace() {
        let a = analyze_spans(fixture_spans(), None, &ReportedDurations::default()).unwrap();
        assert_eq!(a.trace_totals.len(), 2);
        // Trace 1: e2e.test 5000 + GET 200 = 5200 (2 spans); largest first.
        let t1 = &a.trace_totals[0];
        assert_eq!(t1.total_ms, 5200.0);
        assert_eq!(t1.spans, 2);
        // Trace 2: e2e.test 3000 + POST 150 = 3150.
        assert_eq!(a.trace_totals[1].total_ms, 3150.0);
        assert_eq!(a.trace_totals[1].spans, 2);
    }

    #[test]
    fn action_hotspots_from_action_top_json() {
        let a = analyze_spans(fixture_spans(), None, &ReportedDurations::default()).unwrap();
        // "click" appears in both e2e tests (120.5 firefox, 60 chromium); "fill"
        // only in firefox. Sorted by max desc → click, fill.
        assert_eq!(a.action_hotspots.len(), 2);
        let click = &a.action_hotspots[0];
        assert_eq!(click.name, "click");
        assert_eq!(click.count, 2);
        assert_eq!(click.max_ms, 120.5);
        assert_eq!(click.total_ms, 180.5);
        assert_eq!(a.action_hotspots[1].name, "fill");
    }

    #[test]
    fn navigation_phase_and_targets() {
        let a = analyze_spans(fixture_spans(), None, &ReportedDurations::default()).unwrap();
        // Only the Firefox span carries navigation JSON; absence remains optional.
        let total = a
            .navigation_phase_hotspots
            .iter()
            .find(|r| r.name == "navigation.total")
            .expect("navigation.total present");
        assert_eq!(total.count, 2);
        assert_eq!(total.max_ms, 900.0);
        let mount = a
            .navigation_phase_hotspots
            .iter()
            .find(|r| r.name == "navigation.commit_to_mount")
            .expect("commit_to_mount present");
        assert_eq!(mount.max_ms, 400.0);
        // Two navigation targets, feed slowest.
        assert_eq!(a.navigation_targets.len(), 2);
        assert_eq!(a.navigation_targets[0].target, "jaunder.local:8080/feed");
        assert_eq!(a.navigation_targets[0].max_ms, 900.0);
    }

    #[test]
    fn long_tasks_hotspots_and_by_project() {
        let a = analyze_spans(fixture_spans(), None, &ReportedDurations::default()).unwrap();
        // "longtask" in both (90 firefox, 70 chromium); "self" only firefox; the
        // chromium "bad" task (-10) is dropped by the <0 guard.
        let longtask = &a.long_task_hotspots[0];
        assert_eq!(longtask.name, "longtask");
        assert_eq!(longtask.count, 2);
        assert_eq!(longtask.max_ms, 90.0);
        assert!(a.long_task_hotspots.iter().all(|r| r.name != "bad"));
        // By-project (all rows, not sliced): firefox avg-per-test 140 (90+50), then
        // chromium 70.
        assert_eq!(a.long_task_by_project.len(), 2);
        let ff = &a.long_task_by_project[0];
        assert_eq!(ff.project, "firefox");
        assert_eq!(ff.tests, 1);
        assert_eq!(ff.task_count, 2);
        assert_eq!(ff.avg_per_test_ms, 140.0);
        assert_eq!(a.long_task_by_project[1].project, "chromium");
    }

    #[test]
    fn resource_initiators_and_assets() {
        let a = analyze_spans(fixture_spans(), None, &ReportedDurations::default()).unwrap();
        // Initiators: fetch (300) then script (120 + 110), sorted by max desc.
        assert_eq!(a.resource_initiators[0].name, "fetch");
        assert_eq!(a.resource_initiators[0].max_ms, 300.0);
        let script = a
            .resource_initiators
            .iter()
            .find(|r| r.name == "script")
            .expect("script initiator");
        assert_eq!(script.count, 2);
        // Assets keyed by URL path; the wasm asset is slowest, initiator "fetch".
        assert_eq!(a.resource_assets.len(), 3);
        let wasm = &a.resource_assets[0];
        assert_eq!(wasm.name, "jaunder.local:8080/pkg/jaunder.wasm");
        assert_eq!(wasm.initiator, "fetch");
        assert_eq!(wasm.max_ms, 300.0);
    }

    #[test]
    fn analyze_project_filter_over_fixture() {
        // §8: exercise a `--project` run — the `Project filter:` header and the
        // e2e-only filter — end-to-end over the committed fixture.
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
        // Carried for the render header.
        assert_eq!(a.project_filter.as_deref(), Some("firefox"));
        // Only the firefox e2e.test survives; the chromium one is filtered out.
        assert_eq!(a.slowest_e2e_tests.len(), 1);
        assert_eq!(a.slowest_e2e_tests[0].project, "firefox");
        // HTTP spans always pass the project filter (both traces' GET/POST remain).
        assert!(a.slowest_spans.iter().any(|r| r.name == "GET"));
        assert!(a.slowest_spans.iter().any(|r| r.name == "POST"));
        // The report opens with the project-filter header.
        assert!(crate::traces::render::render(&a, 25).starts_with("Project filter: firefox"));
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

use anyhow::Result;
use serde_json::Value;

use super::super::{
    boot_phases,
    parse::{self, Span},
};
use super::model::{AssetRow, BootCoverageRow, HotspotRow, LongTaskProjectRow, TargetRow};

/// Parse an `e2e.*` integer-count attribute (`0` when absent/non-numeric),
/// matching Node's `Number(getAttr(...) || "0")`.
fn count(raw: &Value, key: &str) -> u64 {
    parse::get_attr(raw, key).parse().unwrap_or(0)
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

/// Only the `e2e.test` spans (the ones carrying default-page per-test blobs).
fn e2e_tests(spans: &[Span]) -> impl Iterator<Item = &Span> {
    spans.iter().filter(|s| s.name == "e2e.test")
}

/// Whether `raw` carries the attribute that section-level JSON parsing needs.
fn has_attr(raw: &Value, key: &str) -> bool {
    raw.get("attributes")
        .and_then(Value::as_array)
        .is_some_and(|attrs| {
            attrs
                .iter()
                .any(|attr| attr.get("key").and_then(Value::as_str) == Some(key))
        })
}

/// Spans whose navigation JSON describes document loads for a test-owned page.
fn navigation_bearing_spans(spans: &[Span]) -> impl Iterator<Item = &Span> {
    spans.iter().filter(|s| {
        s.name == "e2e.test"
            || (s.name == "e2e.page" && has_attr(&s.raw, "e2e.navigation_top_json"))
    })
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
pub(super) fn action_hotspot_rows(spans: &[Span]) -> Result<Vec<HotspotRow>> {
    let mut groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let actions = parse::parse_json_attr(&s.raw, "e2e.action_top_json", &s.source)?;
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
/// `e2e.navigation_top_json` on `e2e.test` and `e2e.page` spans. Phases and
/// targets drop negative/non-finite values.
pub(super) fn navigation_sections(spans: &[Span]) -> Result<(Vec<HotspotRow>, Vec<TargetRow>)> {
    let mut phase_groups: Vec<(String, Agg)> = Vec::new();
    let mut url_groups: Vec<(String, Agg)> = Vec::new();
    for s in navigation_bearing_spans(spans) {
        let navs = parse::parse_json_attr(&s.raw, "e2e.navigation_top_json", &s.source)?;
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
            let path = parse::to_url_path(nav.get("url").and_then(Value::as_str).unwrap_or(""));
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
/// `dropped` sums `e2e.navigation_top_dropped` from `e2e.test` and `e2e.page`,
/// because `e2e.navigation_top_json` is the top 20 *by duration* per page sink —
/// a biased sample, not a census. Without it a truncated capture reads as
/// complete coverage.
pub(super) fn boot_coverage_rows(spans: &[Span]) -> Result<Vec<BootCoverageRow>> {
    let mut rows: Vec<BootCoverageRow> = Vec::new();
    for s in navigation_bearing_spans(spans) {
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
                    current: 0,
                    legacy: 0,
                    mounted: 0,
                    full_marks: 0,
                    dropped: 0,
                });
                rows.len() - 1
            }
        };
        let row = &mut rows[idx];
        row.dropped += count(&s.raw, "e2e.navigation_top_dropped");
        let navs = parse::parse_json_attr(&s.raw, "e2e.navigation_top_json", &s.source)?;
        let Some(arr) = navs.as_ref().and_then(Value::as_array) else {
            continue;
        };
        for nav in arr {
            row.navigations += 1;
            let current =
                nav.get("wasmTimingSchema").and_then(Value::as_str) == Some("direct-init-v1");
            if !current {
                row.legacy += 1;
                continue;
            }
            row.current += 1;
            if field_f64(nav, "commitToMountMs").is_some() {
                row.mounted += 1;
            }
            let phases = nav
                .get("bootPhases")
                .and_then(Value::as_object)
                .map_or(0, |phases| phases.len());
            if phases >= boot_phases::MIN_BOOT_PHASES
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
pub(super) fn long_task_sections(
    spans: &[Span],
) -> Result<(Vec<HotspotRow>, Vec<LongTaskProjectRow>)> {
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
        let tasks = parse::parse_json_attr(&s.raw, "e2e.long_tasks_json", &s.source)?;
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
pub(super) fn resource_sections(spans: &[Span]) -> Result<(Vec<HotspotRow>, Vec<AssetRow>)> {
    struct AssetAgg {
        initiator: String,
        agg: Agg,
    }
    let mut init_groups: Vec<(String, Agg)> = Vec::new();
    let mut asset_groups: Vec<(String, AssetAgg)> = Vec::new();
    for s in e2e_tests(spans) {
        let summary = parse::parse_json_attr(&s.raw, "e2e.resource_summary_json", &s.source)?;
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
                .map(parse::to_url_path)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::analyze::analyze_spans;
    use crate::traces::parse::{Filters, parse_spans};
    use crate::traces::report::ReportedDurations;

    const FIXTURE: &str = include_str!("../testdata/otel-traces-sample.jsonl");

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

    fn attr(key: &str, value: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "key": key, "value": value })
    }

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

    fn target_nav(url: &str, total_ms: f64, commit_to_mount_ms: f64) -> serde_json::Value {
        serde_json::json!({
            "url": url,
            "totalMs": total_ms,
            "commitToMountMs": commit_to_mount_ms,
        })
    }

    /// One span carrying `navs` as `e2e.navigation_top_json`, parsed out of the
    /// trace file `source` — the identity `boot_coverage_rows` keys on alongside
    /// the project.
    fn navigation_span(
        span_name: &str,
        source: &str,
        project: &str,
        dropped: u64,
        navs: Vec<serde_json::Value>,
    ) -> Vec<Span> {
        let line = serde_json::json!({
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [{
                        "name": span_name,
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

    fn boot_span(
        source: &str,
        project: &str,
        dropped: u64,
        navs: Vec<serde_json::Value>,
    ) -> Vec<Span> {
        navigation_span("e2e.test", source, project, dropped, navs)
    }

    fn row_for<'a>(rows: &'a [BootCoverageRow], project: &str) -> &'a BootCoverageRow {
        rows.iter()
            .find(|r| r.project == project)
            .unwrap_or_else(|| panic!("no coverage row for {project}"))
    }

    #[test]
    fn boot_coverage_reports_legacy_without_counting_it_as_current_loss() {
        // Firefox carries two pre-cutover navigations. Chromium carries one
        // current full navigation and one legacy navigation.
        let mut legacy_firefox_a = nav(Some(300.0), None, None);
        legacy_firefox_a
            .as_object_mut()
            .unwrap()
            .remove("wasmTimingSchema");
        let mut legacy_firefox_b = nav(Some(280.0), None, None);
        legacy_firefox_b
            .as_object_mut()
            .unwrap()
            .remove("wasmTimingSchema");
        let mut legacy_chromium = nav(None, None, None);
        legacy_chromium
            .as_object_mut()
            .unwrap()
            .remove("wasmTimingSchema");
        let mut spans = boot_span(
            "sqlite",
            "firefox",
            0,
            vec![legacy_firefox_a, legacy_firefox_b],
        );
        spans.extend(boot_span(
            "sqlite",
            "chromium",
            0,
            vec![full_nav(), legacy_chromium],
        ));

        let rows = boot_coverage_rows(&spans).unwrap();
        let ff = row_for(&rows, "firefox");
        assert_eq!(
            (
                ff.navigations,
                ff.current,
                ff.legacy,
                ff.mounted,
                ff.full_marks
            ),
            (2, 0, 2, 0, 0)
        );
        let chr = row_for(&rows, "chromium");
        assert_eq!(
            (
                chr.navigations,
                chr.current,
                chr.legacy,
                chr.mounted,
                chr.full_marks
            ),
            (2, 1, 1, 1, 1)
        );
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
        // `e2e.navigation_top_json` is the top 20 BY DURATION per page sink — a
        // biased sample, not a census. Without the dropped count a truncated capture
        // reads as complete coverage.
        let mut spans = boot_span("sqlite", "firefox", 3, vec![full_nav()]);
        spans.extend(boot_span("sqlite", "firefox", 5, vec![full_nav()]));

        let rows = boot_coverage_rows(&spans).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].navigations, 2);
        assert_eq!(rows[0].dropped, 8, "dropped accumulates across tests");
    }

    #[test]
    fn navigation_sections_include_secondary_page_navigation_json() {
        let mut spans = navigation_span(
            "e2e.test",
            "sqlite",
            "chromium",
            0,
            vec![target_nav("http://jaunder.local:8080/default", 100.0, 40.0)],
        );
        spans.extend(navigation_span(
            "e2e.page",
            "sqlite",
            "chromium",
            0,
            vec![target_nav(
                "http://jaunder.local:8080/secondary",
                250.0,
                90.0,
            )],
        ));

        let (phases, targets) = navigation_sections(&spans).unwrap();

        let total = phases
            .iter()
            .find(|r| r.name == "navigation.total")
            .expect("navigation.total present");
        assert_eq!(total.count, 2);
        assert_eq!(total.total_ms, 350.0);
        assert_eq!(total.max_ms, 250.0);
        assert!(
            targets
                .iter()
                .any(|r| r.target == "jaunder.local:8080/secondary" && r.max_ms == 250.0),
            "secondary-page URL must be present in slow navigation targets",
        );
    }

    #[test]
    fn boot_coverage_reconciles_test_and_page_navigation_sinks() {
        let mut spans = boot_span("sqlite", "firefox", 2, vec![full_nav()]);
        spans.extend(navigation_span(
            "e2e.page",
            "sqlite",
            "firefox",
            3,
            vec![full_nav()],
        ));

        let rows = boot_coverage_rows(&spans).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].navigations, 2);
        assert_eq!(rows[0].dropped, 5);
        assert_eq!(rows[0].current, 2);
        assert_eq!(rows[0].mounted, 2);
        assert_eq!(rows[0].full_marks, 2);
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
}

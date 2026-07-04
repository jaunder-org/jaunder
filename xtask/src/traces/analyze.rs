//! Trace analysis: `Vec<Span>` → a typed [`Analysis`] of every report section.
//!
//! The reusable in-crate seam (ADR-0028 host analyzer). `analyze_spans` does the
//! whole computation with no I/O, so #33's `traces run` can call it in-process on
//! trace files it collected, and unit tests can drive it from a fixture without
//! spawning a process. Rows are **fully sorted** here; `--top` slicing is
//! [`super::render`]'s job. Port of the twelve `print*` functions in
//! `scripts/analyze-otel-traces`.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;

use super::parse::{get_attr, parse_json_attr, read_spans, to_url_path, Filters, Span};

/// Parse an `e2e.*` integer-count attribute (`0` when absent/non-numeric),
/// matching Node's `Number(getAttr(...) || "0")`.
fn count(raw: &Value, key: &str) -> u64 {
    get_attr(raw, key).parse().unwrap_or(0)
}

/// A finite `f64` field of a JSON object, else `None` (Node's `asFiniteNumber` /
/// `Number.isFinite` guards).
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
    if t.is_empty() {
        "-".to_string()
    } else {
        t
    }
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
    /// Navigation commit→hydration by cache warmth, `avg_ms` desc. All rows. Section 5.
    pub cache_warmth: Vec<CacheWarmthRow>,
    /// Per-test hydration budget vs API budget, `hydration_ms` desc. Section 8.
    pub hydration_vs_api: Vec<HydrationVsApiRow>,
    /// Navigation phase component samples, `ms` desc. Section 9a.
    pub nav_phase_component_samples: Vec<PhaseSampleRow>,
    /// Navigation phase component targets, `max_ms` desc. Section 9b.
    pub nav_phase_component_targets: Vec<PhaseTargetRow>,
    /// Navigation phase components by project, `avg_ms` desc. Section 9c.
    pub nav_phase_component_by_project: Vec<PhaseProjectRow>,
    /// Hydration runtime component samples, `ms` desc. Section 10a.
    pub hydration_runtime_samples: Vec<RuntimeSampleRow>,
    /// Hydration runtime components by project, `avg_ms` desc. Section 10b.
    pub hydration_runtime_by_project: Vec<RuntimeProjectRow>,
}

/// One row of the "slowest spans" table (Node `printSlowest` :189-214).
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

/// One row of "slowest e2e.test spans" (Node `printSlowestE2eTests` :216-249).
#[derive(Debug, Clone)]
pub struct E2eTestRow {
    pub duration_ms: f64,
    pub project: String,
    pub actions: u64,
    pub requests: u64,
    pub trace_id: String,
    pub test: String,
}

/// One row of "E2E test duration by project" (Node `printE2eByProject` :1017-1067).
#[derive(Debug, Clone)]
pub struct ByProjectRow {
    pub project: String,
    pub tests: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub avg_actions: f64,
    pub avg_requests: f64,
}

/// One row of "Trace totals" (Node `printTraceTotals` :1070-1096).
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

/// Navigation commit→hydration by cache warmth (Node `printE2eNavigationCacheWarmth`).
#[derive(Debug, Clone)]
pub struct CacheWarmthRow {
    pub cache_warmth: String,
    pub project: String,
    pub count: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
}

/// A test's navigation-hydration budget vs API budget (Node `printE2eHydrationVsApi`).
#[derive(Debug, Clone)]
pub struct HydrationVsApiRow {
    pub hydration_ms: f64,
    pub api_ms: f64,
    pub ratio: Option<f64>,
    pub project: String,
    pub trace_id: String,
    pub test: String,
}

/// A single navigation-phase-component sample (Node phase-component `samples`).
#[derive(Debug, Clone)]
pub struct PhaseSampleRow {
    pub phase: String,
    pub project: String,
    pub ms: f64,
    pub trace_id: String,
    pub target: String,
}

/// Navigation phase component aggregated by (phase, project, target).
#[derive(Debug, Clone)]
pub struct PhaseTargetRow {
    pub phase: String,
    pub project: String,
    pub target: String,
    pub count: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
}

/// Navigation phase component aggregated by (phase, project).
#[derive(Debug, Clone)]
pub struct PhaseProjectRow {
    pub phase: String,
    pub project: String,
    pub count: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
}

/// A single hydration-runtime-component sample (Node `printHydrationRuntimeComponents`).
#[derive(Debug, Clone)]
pub struct RuntimeSampleRow {
    pub component: String,
    pub project: String,
    pub ms: f64,
    pub trace_id: String,
    pub test: String,
}

/// Hydration runtime component aggregated by (component, project).
#[derive(Debug, Clone)]
pub struct RuntimeProjectRow {
    pub component: String,
    pub project: String,
    pub count: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
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
fn action_hotspot_rows(spans: &[Span]) -> Vec<HotspotRow> {
    let mut groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let actions = parse_json_attr(&s.raw, "e2e.action_top_json");
        let Some(arr) = actions.as_array() else {
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
    hotspot_rows(groups)
}

/// The navigation phase fields aggregated as `navigation.<label>` (Node `addPhase`
/// set, :373-387).
const NAV_PHASES: [(&str, &str); 9] = [
    ("navigation.total", "totalMs"),
    ("navigation.request", "requestMs"),
    (
        "navigation.commit_to_domcontentloaded",
        "commitToDomContentLoadedMs",
    ),
    ("navigation.commit_to_hydration", "commitToHydrationMs"),
    (
        "navigation.domcontentloaded_to_load",
        "domContentLoadedToLoadMs",
    ),
    ("navigation.load_to_hydration", "loadToHydrationMs"),
    ("navigation.wasm_init", "wasmInitMs"),
    ("navigation.leptos_hydrate", "leptosHydrateMs"),
    ("navigation.post_hydrate_effects", "postHydrateEffectsMs"),
];

/// Section 4 — navigation phase totals + slow navigation targets from
/// `e2e.navigation_top_json`. Phases and targets drop negative/non-finite values.
fn navigation_sections(spans: &[Span]) -> (Vec<HotspotRow>, Vec<TargetRow>) {
    let mut phase_groups: Vec<(String, Agg)> = Vec::new();
    let mut url_groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let navs = parse_json_attr(&s.raw, "e2e.navigation_top_json");
        let Some(arr) = navs.as_array() else {
            continue;
        };
        for nav in arr {
            for (label, field) in NAV_PHASES {
                if let Some(v) = field_f64(nav, field) {
                    if v >= 0.0 {
                        entry(&mut phase_groups, label, Agg::default).add(v);
                    }
                }
            }
            let path = to_url_path(nav.get("url").and_then(Value::as_str).unwrap_or(""));
            if path.is_empty() {
                continue;
            }
            if let Some(total) = field_f64(nav, "totalMs") {
                if total >= 0.0 {
                    entry(&mut url_groups, &path, Agg::default).add(total);
                }
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
    (phase_rows, target_rows)
}

/// Section 6 — long-task hotspots by task name + per-project totals from
/// `e2e.long_tasks_json`. Negative/non-finite durations are dropped.
fn long_task_sections(spans: &[Span]) -> (Vec<HotspotRow>, Vec<LongTaskProjectRow>) {
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
        let tasks = parse_json_attr(&s.raw, "e2e.long_tasks_json");
        let Some(arr) = tasks.as_array() else {
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
    (hotspots, project_rows)
}

/// Section 7 — resource initiator hotspots + slow assets from
/// `e2e.resource_summary_json.topSlow`. Negative/non-finite durations dropped.
fn resource_sections(spans: &[Span]) -> (Vec<HotspotRow>, Vec<AssetRow>) {
    struct AssetAgg {
        initiator: String,
        agg: Agg,
    }
    let mut init_groups: Vec<(String, Agg)> = Vec::new();
    let mut asset_groups: Vec<(String, AssetAgg)> = Vec::new();
    for s in e2e_tests(spans) {
        let summary = parse_json_attr(&s.raw, "e2e.resource_summary_json");
        let Some(items) = summary.get("topSlow").and_then(Value::as_array) else {
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
    (initiator_rows, asset_rows)
}

/// Section 5 — navigation commit→hydration grouped by cache warmth × project.
fn cache_warmth_rows(spans: &[Span]) -> Vec<CacheWarmthRow> {
    let mut groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let navs = parse_json_attr(&s.raw, "e2e.navigation_top_json");
        let Some(arr) = navs.as_array() else {
            continue;
        };
        for nav in arr {
            let Some(hyd) = field_f64(nav, "commitToHydrationMs") else {
                continue;
            };
            if hyd < 0.0 {
                continue;
            }
            let warmth = if nav.get("cacheWarmth").and_then(Value::as_str) == Some("warm") {
                "warm"
            } else {
                "cold"
            };
            let key = format!("{warmth}\n{}", project_label(&s.project));
            entry(&mut groups, &key, Agg::default).add(hyd);
        }
    }
    let mut rows: Vec<CacheWarmthRow> = groups
        .into_iter()
        .map(|(key, a)| {
            let (warmth, project) = key.split_once('\n').unwrap_or(("", ""));
            CacheWarmthRow {
                cache_warmth: warmth.to_string(),
                project: project.to_string(),
                count: a.count,
                avg_ms: a.avg(),
                max_ms: a.max_ms,
            }
        })
        .collect();
    sort_desc_by(&mut rows, |r| r.avg_ms);
    rows
}

/// Section 8 — per-test navigation-hydration budget vs `/api/` request budget.
fn hydration_vs_api_rows(spans: &[Span]) -> Vec<HydrationVsApiRow> {
    let mut rows: Vec<HydrationVsApiRow> = Vec::new();
    for s in e2e_tests(spans) {
        let mut hydration_ms = 0.0;
        if let Some(arr) = parse_json_attr(&s.raw, "e2e.navigation_top_json").as_array() {
            for nav in arr {
                if let Some(v) = field_f64(nav, "commitToHydrationMs") {
                    if v > 0.0 {
                        hydration_ms += v;
                    }
                }
            }
        }
        let mut api_ms = 0.0;
        if let Some(arr) = parse_json_attr(&s.raw, "e2e.request_top_slow_json").as_array() {
            for req in arr {
                let url = req.get("url").and_then(Value::as_str).unwrap_or("");
                if !url.contains("/api/") {
                    continue;
                }
                if let Some(v) = field_f64(req, "durationMs") {
                    if v > 0.0 {
                        api_ms += v;
                    }
                }
            }
        }
        let ratio = if api_ms > 0.0 {
            Some(hydration_ms / api_ms)
        } else {
            None
        };
        rows.push(HydrationVsApiRow {
            hydration_ms,
            api_ms,
            ratio,
            project: project_label(&s.project),
            trace_id: s.trace_id.clone(),
            test: e2e_test_name(s),
        });
    }
    sort_desc_by(&mut rows, |r| r.hydration_ms);
    rows
}

/// The navigation-phase component fields (Node `printNavigationPhaseComponentHotspots`).
const NAV_PHASE_COMPONENTS: [(&str, &str); 4] = [
    ("commit_to_hydration", "commitToHydrationMs"),
    ("wasm_init", "wasmInitMs"),
    ("leptos_hydrate", "leptosHydrateMs"),
    ("post_hydrate_effects", "postHydrateEffectsMs"),
];

/// Section 9 — navigation phase component samples, targets, and by-project.
fn nav_phase_component_sections(
    spans: &[Span],
) -> (
    Vec<PhaseSampleRow>,
    Vec<PhaseTargetRow>,
    Vec<PhaseProjectRow>,
) {
    let mut samples: Vec<PhaseSampleRow> = Vec::new();
    let mut target_groups: Vec<(String, Agg)> = Vec::new();
    let mut proj_groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let project = project_label(&s.project);
        let navs = parse_json_attr(&s.raw, "e2e.navigation_top_json");
        let Some(arr) = navs.as_array() else {
            continue;
        };
        for nav in arr {
            let target = to_url_path(nav.get("url").and_then(Value::as_str).unwrap_or("-"));
            for (label, field) in NAV_PHASE_COMPONENTS {
                let Some(v) = field_f64(nav, field) else {
                    continue;
                };
                if v < 0.0 {
                    continue;
                }
                samples.push(PhaseSampleRow {
                    phase: label.to_string(),
                    project: project.clone(),
                    ms: v,
                    trace_id: s.trace_id.clone(),
                    target: target.clone(),
                });
                entry(
                    &mut target_groups,
                    &format!("{label}\n{project}\n{target}"),
                    Agg::default,
                )
                .add(v);
                entry(
                    &mut proj_groups,
                    &format!("{label}\n{project}"),
                    Agg::default,
                )
                .add(v);
            }
        }
    }
    sort_desc_by(&mut samples, |r| r.ms);
    let mut targets: Vec<PhaseTargetRow> = target_groups
        .into_iter()
        .map(|(key, a)| {
            let p: Vec<&str> = key.splitn(3, '\n').collect();
            PhaseTargetRow {
                phase: p[0].to_string(),
                project: p[1].to_string(),
                target: p[2].to_string(),
                count: a.count,
                avg_ms: a.avg(),
                max_ms: a.max_ms,
            }
        })
        .collect();
    sort_desc_by(&mut targets, |r| r.max_ms);
    let mut by_project: Vec<PhaseProjectRow> = proj_groups
        .into_iter()
        .map(|(key, a)| {
            let (phase, project) = key.split_once('\n').unwrap_or(("", ""));
            PhaseProjectRow {
                phase: phase.to_string(),
                project: project.to_string(),
                count: a.count,
                avg_ms: a.avg(),
                max_ms: a.max_ms,
            }
        })
        .collect();
    sort_desc_by(&mut by_project, |r| r.avg_ms);
    (samples, targets, by_project)
}

/// The hydration-runtime component fields (Node `printHydrationRuntimeComponents`).
const HYDRATION_RUNTIME_COMPONENTS: [(&str, &str); 5] = [
    ("hydration", "hydrationMs"),
    ("wasm_resource", "wasmResourceMs"),
    ("wasm_init", "wasmInitMs"),
    ("leptos_hydrate", "leptosHydrateMs"),
    ("post_hydrate_effects", "postHydrateEffectsMs"),
];

/// Section 10 — hydration runtime component samples and by-project.
fn hydration_runtime_sections(spans: &[Span]) -> (Vec<RuntimeSampleRow>, Vec<RuntimeProjectRow>) {
    let mut samples: Vec<RuntimeSampleRow> = Vec::new();
    let mut proj_groups: Vec<(String, Agg)> = Vec::new();
    for s in e2e_tests(spans) {
        let project = project_label(&s.project);
        let test = e2e_test_name(s);
        let runtime = parse_json_attr(&s.raw, "e2e.hydration_runtime_json");
        if !runtime.is_object() {
            continue;
        }
        for (label, field) in HYDRATION_RUNTIME_COMPONENTS {
            let Some(v) = field_f64(&runtime, field) else {
                continue;
            };
            if v < 0.0 {
                continue;
            }
            samples.push(RuntimeSampleRow {
                component: label.to_string(),
                project: project.clone(),
                ms: v,
                trace_id: s.trace_id.clone(),
                test: test.clone(),
            });
            entry(
                &mut proj_groups,
                &format!("{label}\n{project}"),
                Agg::default,
            )
            .add(v);
        }
    }
    sort_desc_by(&mut samples, |r| r.ms);
    let mut by_project: Vec<RuntimeProjectRow> = proj_groups
        .into_iter()
        .map(|(key, a)| {
            let (component, project) = key.split_once('\n').unwrap_or(("", ""));
            RuntimeProjectRow {
                component: component.to_string(),
                project: project.to_string(),
                count: a.count,
                avg_ms: a.avg(),
                max_ms: a.max_ms,
            }
        })
        .collect();
    sort_desc_by(&mut by_project, |r| r.avg_ms);
    (samples, by_project)
}

/// Compute the whole [`Analysis`] from already-parsed spans. No I/O.
pub fn analyze_spans(spans: Vec<Span>, project_filter: Option<String>) -> Analysis {
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
    let action_hotspots = action_hotspot_rows(&spans);
    let (navigation_phase_hotspots, navigation_targets) = navigation_sections(&spans);
    let (long_task_hotspots, long_task_by_project) = long_task_sections(&spans);
    let (resource_initiators, resource_assets) = resource_sections(&spans);

    // Sections 5, 8, 9, 10 — cache warmth, hydration budgets, phase/runtime components.
    let cache_warmth = cache_warmth_rows(&spans);
    let hydration_vs_api = hydration_vs_api_rows(&spans);
    let (nav_phase_component_samples, nav_phase_component_targets, nav_phase_component_by_project) =
        nav_phase_component_sections(&spans);
    let (hydration_runtime_samples, hydration_runtime_by_project) =
        hydration_runtime_sections(&spans);

    Analysis {
        span_count: spans.len(),
        project_filter,
        slowest_spans,
        slowest_e2e_tests,
        by_project,
        trace_totals,
        action_hotspots,
        navigation_phase_hotspots,
        navigation_targets,
        long_task_hotspots,
        long_task_by_project,
        resource_initiators,
        resource_assets,
        cache_warmth,
        hydration_vs_api,
        nav_phase_component_samples,
        nav_phase_component_targets,
        nav_phase_component_by_project,
        hydration_runtime_samples,
        hydration_runtime_by_project,
    }
}

/// Read + parse every input, then analyze. `filters.project` is carried into
/// `Analysis.project_filter` for the render header.
pub fn analyze(inputs: &[PathBuf], filters: Filters) -> Result<Analysis> {
    let mut spans = Vec::new();
    for input in inputs {
        spans.extend(read_spans(input, &filters)?);
    }
    Ok(analyze_spans(spans, filters.project))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::parse::parse_spans;

    const FIXTURE: &str = include_str!("testdata/otel-traces-sample.jsonl");

    fn fixture_spans() -> Vec<Span> {
        parse_spans(FIXTURE, &Filters::default(), "sample").unwrap()
    }

    #[test]
    fn slowest_spans_sorted_desc_and_complete() {
        let spans = fixture_spans();
        let n = spans.len();
        assert!(n > 0, "fixture must have spans");
        let a = analyze_spans(spans, None);
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
        let a = analyze_spans(fixture_spans(), None);
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
        let a = analyze_spans(fixture_spans(), None);
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
        let a = analyze_spans(fixture_spans(), None);
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
        let a = analyze_spans(fixture_spans(), None);
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
        let a = analyze_spans(fixture_spans(), None);
        // Only the firefox span has valid navigation JSON (chromium's is malformed).
        let total = a
            .navigation_phase_hotspots
            .iter()
            .find(|r| r.name == "navigation.total")
            .expect("navigation.total present");
        assert_eq!(total.count, 2);
        assert_eq!(total.max_ms, 900.0);
        let hyd = a
            .navigation_phase_hotspots
            .iter()
            .find(|r| r.name == "navigation.commit_to_hydration")
            .expect("commit_to_hydration present");
        assert_eq!(hyd.max_ms, 400.0);
        // Two navigation targets, feed slowest.
        assert_eq!(a.navigation_targets.len(), 2);
        assert_eq!(a.navigation_targets[0].target, "jaunder.local:8080/feed");
        assert_eq!(a.navigation_targets[0].max_ms, 900.0);
    }

    #[test]
    fn long_tasks_hotspots_and_by_project() {
        let a = analyze_spans(fixture_spans(), None);
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
        let a = analyze_spans(fixture_spans(), None);
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
        assert_eq!(wasm.name, "jaunder.local:8080/pkg/jaunder_bg.wasm");
        assert_eq!(wasm.initiator, "fetch");
        assert_eq!(wasm.max_ms, 300.0);
    }

    #[test]
    fn cache_warmth_by_warmth_and_project() {
        let a = analyze_spans(fixture_spans(), None);
        // Firefox nav has a cold (commit→hydration 400) and a warm (220) entry;
        // chromium's nav JSON is malformed, so it contributes nothing.
        assert_eq!(a.cache_warmth.len(), 2);
        // Sorted by avg desc → cold (400) first.
        assert_eq!(a.cache_warmth[0].cache_warmth, "cold");
        assert_eq!(a.cache_warmth[0].project, "firefox");
        assert_eq!(a.cache_warmth[0].avg_ms, 400.0);
        let warm = a
            .cache_warmth
            .iter()
            .find(|r| r.cache_warmth == "warm")
            .expect("warm row");
        assert_eq!(warm.avg_ms, 220.0);
    }

    #[test]
    fn hydration_vs_api_budget() {
        let a = analyze_spans(fixture_spans(), None);
        assert_eq!(a.hydration_vs_api.len(), 2);
        // Firefox: hydration 400+220=620; api = create_post 210 (feed has no /api/).
        let ff = &a.hydration_vs_api[0];
        assert_eq!(ff.project, "firefox");
        assert_eq!(ff.hydration_ms, 620.0);
        assert_eq!(ff.api_ms, 210.0);
        assert!((ff.ratio.unwrap() - 620.0 / 210.0).abs() < 1e-9);
        // Chromium: no valid nav hydration → 0; api = get_feed 80; ratio = 0.
        let ch = &a.hydration_vs_api[1];
        assert_eq!(ch.hydration_ms, 0.0);
        assert_eq!(ch.api_ms, 80.0);
        assert_eq!(ch.ratio, Some(0.0));
    }

    #[test]
    fn navigation_phase_components() {
        let a = analyze_spans(fixture_spans(), None);
        // Slowest sample is firefox commit_to_hydration = 400.
        let top = &a.nav_phase_component_samples[0];
        assert_eq!(top.phase, "commit_to_hydration");
        assert_eq!(top.ms, 400.0);
        // By-project: commit_to_hydration/firefox over 400 & 220 → count 2, avg 310.
        let cth = a
            .nav_phase_component_by_project
            .iter()
            .find(|r| r.phase == "commit_to_hydration" && r.project == "firefox")
            .expect("commit_to_hydration/firefox");
        assert_eq!(cth.count, 2);
        assert_eq!(cth.avg_ms, 310.0);
        assert_eq!(cth.max_ms, 400.0);
        // Targets carry the URL path.
        assert!(a.nav_phase_component_targets.iter().any(|r| {
            r.phase == "commit_to_hydration"
                && r.target == "jaunder.local:8080/feed"
                && r.max_ms == 400.0
        }));
    }

    #[test]
    fn hydration_runtime_components() {
        let a = analyze_spans(fixture_spans(), None);
        // Slowest sample is firefox hydration = 420.
        let top = &a.hydration_runtime_samples[0];
        assert_eq!(top.component, "hydration");
        assert_eq!(top.ms, 420.0);
        let h_ff = a
            .hydration_runtime_by_project
            .iter()
            .find(|r| r.component == "hydration" && r.project == "firefox")
            .expect("hydration/firefox");
        assert_eq!(h_ff.avg_ms, 420.0);
        // Firefox reports wasm_resource; chromium's runtime omits it.
        assert!(a
            .hydration_runtime_by_project
            .iter()
            .any(|r| r.component == "wasm_resource" && r.project == "firefox"));
        assert!(!a
            .hydration_runtime_by_project
            .iter()
            .any(|r| r.component == "wasm_resource" && r.project == "chromium"));
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
        let a = analyze_spans(spans, filters.project.clone());
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
    fn analyze_reads_files() {
        let dir = std::env::temp_dir().join(format!("traces-analyze-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("otel-traces.jsonl");
        std::fs::write(&file, FIXTURE).unwrap();

        let via_file = analyze(&[file], Filters::default()).unwrap();
        let via_spans = analyze_spans(fixture_spans(), None);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(via_file.span_count, via_spans.span_count);
        assert_eq!(via_file.slowest_spans.len(), via_spans.slowest_spans.len());
        assert_eq!(
            via_file.slowest_spans.first().map(|r| r.duration_ms),
            via_spans.slowest_spans.first().map(|r| r.duration_ms),
        );
    }
}

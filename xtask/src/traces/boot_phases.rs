//! `cargo xtask traces boot-phases` — median boot-phase decomposition per
//! `(source, project, cacheWarmth)` (#818, spec D8/AC18).
//!
//! `analyze` computes maxima and means; AC13 is entirely medians, so this is its
//! own command rather than another section there.
//!
//! **The frame rule (spec D8).** Every decomposed segment is document-relative
//! (`performance.timeOrigin`), so the decomposition target is
//! `bootTotalMs := mount_done.startTime`, never `commitToMountMs` — that one is
//! `mountedMs - committedMs`, both Node-side `Date.now()`. Mixing the two frames
//! is the error #794 shipped: it charges CDP/juggler event-delivery latency to the
//! app's boot phases. `commitToMountMs` and the frame skew between the frames are
//! therefore **reported and never decomposed**.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;
use tabled::settings::Style;
use tabled::{Table, Tabled};

use super::parse::{Filters, Span, parse_json_attr, read_spans};

/// Spec D5's two exclusive pre-Rust-boot segments. Resource delivery and direct
/// initializer timings are diagnostics; neither closes the boot decomposition.
const EXCLUSIVE_SEGMENTS: [(&str, &str); 2] = [
    ("wasm_init_start", "wasmInitStartMs"),
    (
        "wasm_init_start_to_boot_entry",
        "wasmInitStartToBootEntryMs",
    ),
];

/// The fewest finite `boot.*` intervals the document-frame decomposition itself
/// uses. It is a floor so future instrumentation can add marks without becoming
/// a coverage blackout.
pub(crate) const MIN_BOOT_PHASES: usize = 3;

/// How far the segments may miss `mount_done.startTime` before the navigation is
/// counted as a closure violation instead of a sample. The segments close
/// *exactly* by construction, so this is float slack, not a tolerance band.
const CLOSURE_TOLERANCE_MS: f64 = 1.0;

/// The mark that ends the boot total. Matched by suffix — the names live in Rust
/// (`client::perf`), and only the tail is contractual.
const MOUNT_DONE_SUFFIX: &str = ".boot.mount_done";

/// A `bootPhases` key belongs to the boot decomposition iff it mentions a boot
/// mark. **Selected by substring, never by position or count:** `bootPhasesFrom`
/// (`end2end/tests/fixtures.ts`) emits n-1 intervals over *all* `jaunder.*` marks,
/// so a new mark must extend the decomposition rather than break its closure.
const BOOT_INTERVAL_KEY: &str = "boot.";
/// Shared bridge schema for frame-skew diagnostics recorded on navigations.
const BRIDGE_SCHEMA: &str = "bridge-v1";

/// Remainder closure tolerance registered by issue #868. Populations that miss
/// it stay non-decisive.
const BRIDGE_MEAN_TOLERANCE_MS: f64 = 1.0;
const BRIDGE_MAX_TOLERANCE_MS: f64 = 2.0;

/// One segment's median across a population.
///
/// `count` is carried because segment labels are discovered per navigation: a
/// label present on only some of them is a data defect, and a bare median would
/// hide it.
#[derive(Debug, Clone)]
pub struct SegmentMedian {
    pub label: String,
    pub median_ms: f64,
    pub count: usize,
}

/// One `(source, project, cacheWarmth, experiment arm)` population.
///
/// `navigations` is every navigation seen, `decomposed` only those that carried a
/// full mark set *and* closed. The two differing is the #818 signal itself, so
/// both are reported rather than one implied from the other.
#[derive(Debug, Clone)]
pub struct BootPhaseRow {
    /// The trace file. **Load-bearing:** `projectName` is the browser and names no
    /// backend (`traces/run.rs:99-101`), so without it sqlite pools with postgres.
    pub source: String,
    pub project: String,
    pub cache_warmth: String,
    pub experiment_arm: String,
    pub navigations: usize,
    pub decomposed: usize,
    pub current: usize,
    pub mounted: usize,
    pub bridge_complete: usize,
    pub bridge_decisive: bool,
    pub direct_complete: usize,
    pub direct_missing: usize,
    pub streaming: usize,
    pub buffered: usize,
    pub legacy: usize,
    pub wasm_api_ms: Option<f64>,
    pub wasm_init_ms: Option<f64>,
    pub shape_complete: usize,
    pub shape_missing: usize,
    pub wasm_api_mean_ms: Option<f64>,
    pub wasm_api_se_ms: Option<f64>,
    pub wasm_init_mean_ms: Option<f64>,
    pub wasm_init_se_ms: Option<f64>,
    pub commit_to_document_start_mean_ms: Option<f64>,
    pub commit_to_document_start_se_ms: Option<f64>,
    pub mount_done_to_binding_mean_ms: Option<f64>,
    pub mount_done_to_binding_se_ms: Option<f64>,
    pub frame_skew_remainder_mean_ms: Option<f64>,
    pub frame_skew_remainder_se_ms: Option<f64>,
    pub frame_skew_remainder_max_abs_ms: Option<f64>,
    pub style_complete: usize,
    pub style_decisive: bool,
    pub style_ordering_pass: usize,
    pub style_to_module_before_init_mean_ms: Option<f64>,
    pub style_to_module_before_init_se_ms: Option<f64>,
    pub module_before_init_to_wasm_fetch_start_mean_ms: Option<f64>,
    pub module_before_init_to_wasm_fetch_start_se_ms: Option<f64>,
    pub style_max_response_end_mean_ms: Option<f64>,
    pub style_max_response_end_se_ms: Option<f64>,
    pub wasm_fetch_start_mean_ms: Option<f64>,
    pub wasm_fetch_start_se_ms: Option<f64>,
    pub unique_shapes: usize,
    /// Navigations whose segments missed `mount_done.startTime` by more than
    /// [`CLOSURE_TOLERANCE_MS`]. Excluded from every median and counted here —
    /// never silently included, never silently dropped.
    pub closure_violations: usize,
    /// Spec D8's segments in `startTime` order, medians over `decomposed`.
    pub segments: Vec<SegmentMedian>,
    /// Median `mount_done.startTime` — the decomposition target.
    pub boot_total_ms: Option<f64>,
    /// Median Node-side wall clock. Reported as the bridge to the gate's figures,
    /// never decomposed (spec D8).
    pub commit_to_mount_ms: Option<f64>,
    /// Median `commitToMountMs - bootTotalMs`: harness overhead, bidirectional,
    /// and its own row rather than a segment (spec D8, AC14).
    pub frame_skew_ms: Option<f64>,
}

/// Median with **the lower of the two middle values on an even count**.
///
/// Pinned by a test: `traces analyze` computes no medians, so there is no
/// in-repo convention to inherit and an unstated one would be re-guessed by the
/// next reader.
fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[(values.len() - 1) / 2])
}

/// A finite `f64` field of a JSON object, else `None`.
fn field_f64(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}

fn mean_and_standard_error(values: &[f64]) -> (Option<f64>, Option<f64>) {
    if values.is_empty() {
        return (None, None);
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    if values.len() < 2 {
        return (Some(mean), None);
    }
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / (n - 1.0);
    (Some(mean), Some(variance.sqrt() / n.sqrt()))
}
#[derive(Debug, Clone)]
struct BridgeSample {
    commit_to_document_start_ms: f64,
    mount_done_to_binding_ms: f64,
    frame_skew_remainder_ms: f64,
}
#[derive(Debug, Clone)]
struct StyleSample {
    style_max_response_end_ms: f64,
    style_to_module_before_init_ms: f64,
    module_before_init_to_wasm_fetch_start_ms: f64,
    wasm_fetch_start_ms: f64,
}

/// One navigation's decomposition: the ordered segments and the reported
/// (never decomposed) wall-clock quantities.
#[derive(Debug, Clone)]
struct Decomposition {
    segments: Vec<(String, f64)>,
    boot_total_ms: f64,
    commit_to_mount_ms: Option<f64>,
}

/// What one navigation contributed.
enum NavOutcome {
    /// No full mark set — the #818 blackout, counted but not measurable.
    NotDecomposed,
    /// A full mark set whose segments do not sum to the boot total.
    ClosureViolation,
    Decomposed(Box<Decomposition>),
}

/// Whether a navigation has a complete, closing document-frame decomposition.
///
/// The evaluator keeps this classification alongside the median renderer so
/// both consumers apply the same schema and closure calculation.
pub(crate) enum BootDecompositionOutcome {
    Incomplete,
    ClosureViolation,
    Complete,
}

/// `name -> startTime` for one navigation's marks.
fn mark_starts(marks: &[Value]) -> BTreeMap<String, f64> {
    marks
        .iter()
        .filter_map(|mark| {
            let name = mark.get("name").and_then(Value::as_str)?;
            Some((name.to_string(), field_f64(mark, "startTime")?))
        })
        .collect()
}

/// A complete stylesheet/module diagnostic sample from one navigation. Historical
/// traces simply omit these keys; that is non-decisive, not an error.
fn style_sample(nav: &Value) -> Option<StyleSample> {
    let _module_before_init_ms = field_f64(nav, "moduleBeforeInitMs")?;
    let _jaunder_css_response_end_ms = field_f64(nav, "jaunderCssResponseEndMs")?;
    let _jaunder_themes_css_response_end_ms = field_f64(nav, "jaunderThemesCssResponseEndMs")?;
    Some(StyleSample {
        style_max_response_end_ms: field_f64(nav, "styleMaxResponseEndMs")?,
        style_to_module_before_init_ms: field_f64(nav, "styleToModuleBeforeInitMs")?,
        module_before_init_to_wasm_fetch_start_ms: field_f64(
            nav,
            "moduleBeforeInitToWasmFetchStartMs",
        )?,
        wasm_fetch_start_ms: field_f64(nav, "wasmFetchStartMs")?,
    })
}

/// A complete bridge diagnostic sample from one mounted navigation. Historical
/// traces simply omit these keys; that is non-decisive, not an error.
fn bridge_sample(nav: &Value) -> Option<BridgeSample> {
    let schema = nav.get("frameSkewSchema")?.as_str()?;
    if schema != BRIDGE_SCHEMA {
        return None;
    }
    Some(BridgeSample {
        commit_to_document_start_ms: field_f64(nav, "commitToDocumentStartMs")?,
        mount_done_to_binding_ms: field_f64(nav, "mountDoneToBindingMs")?,
        frame_skew_remainder_ms: field_f64(nav, "frameSkewRemainderMs")?,
    })
}

/// Decompose one current navigation against its own marks (spec D5).
///
/// `page_total` is only supplied by the explicit page-span classification path;
/// ordinary analysis and test-span evidence must close to the mark-derived end.
fn decompose(nav: &Value, marks: &[Value], page_total: Option<f64>) -> NavOutcome {
    if nav.get("wasmTimingSchema").and_then(Value::as_str) != Some("direct-init-v1") {
        return NavOutcome::NotDecomposed;
    }
    let starts = mark_starts(marks);
    let boot_total_ms = starts
        .iter()
        .find(|(name, _)| name.ends_with(MOUNT_DONE_SUFFIX))
        .map(|(_, start)| *start)
        .or(page_total);
    let Some(boot_total_ms) = boot_total_ms else {
        return NavOutcome::NotDecomposed;
    };
    let mut segments: Vec<(String, f64)> = Vec::new();

    for (label, field) in EXCLUSIVE_SEGMENTS {
        let Some(value) = field_f64(nav, field) else {
            return NavOutcome::NotDecomposed;
        };
        segments.push((label.to_string(), value));
    }

    let Some(phases) = nav.get("bootPhases").and_then(Value::as_object) else {
        return NavOutcome::NotDecomposed;
    };
    let mut intervals: Vec<(String, f64)> = phases
        .iter()
        .filter(|(key, _)| key.contains(BOOT_INTERVAL_KEY))
        .filter_map(|(key, value)| {
            let ms = value.as_f64().filter(|n| n.is_finite())?;
            Some((key.clone(), ms))
        })
        .collect();
    if intervals.len() < MIN_BOOT_PHASES {
        return NavOutcome::NotDecomposed;
    }
    // `startTime` order, from the marks themselves: `bootPhases` is a JSON object,
    // so its own key order is not a timeline.
    //
    // Ordered on BOTH endpoints, because sub-millisecond phases really do report
    // as zero-length: two intervals then share a `from` startTime and the object's
    // key order — alphabetical, not chronological — would decide, printing the
    // segments out of sequence in exactly the populations where boot is fastest.
    intervals.sort_by(|left, right| {
        let bounds = |key: &str| {
            let start = |name: &str| starts.get(name).copied().unwrap_or(f64::INFINITY);
            let mut ends = key.split("->");
            let from = ends.next().unwrap_or(key);
            (start(from), ends.next().map_or(f64::INFINITY, start))
        };
        let (left_from, left_to) = bounds(&left.0);
        let (right_from, right_to) = bounds(&right.0);
        left_from
            .partial_cmp(&right_from)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                left_to
                    .partial_cmp(&right_to)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    segments.extend(intervals);

    let sum: f64 = segments.iter().map(|(_, value)| value).sum();
    if (sum - boot_total_ms).abs() > CLOSURE_TOLERANCE_MS {
        return NavOutcome::ClosureViolation;
    }
    NavOutcome::Decomposed(Box::new(Decomposition {
        segments,
        boot_total_ms,
        commit_to_mount_ms: field_f64(nav, "commitToMountMs"),
    }))
}

/// Classify test-span navigation evidence using mark-derived document closure.
pub(crate) fn boot_decomposition_outcome(
    navigation: &Value,
    marks: &[Value],
) -> BootDecompositionOutcome {
    outcome(decompose(navigation, marks, None))
}

/// Classify page-span navigation evidence, which may carry its document total
/// without the test span's boot-mark attribute.
pub(crate) fn page_boot_decomposition_outcome(
    navigation: &Value,
    marks: &[Value],
) -> BootDecompositionOutcome {
    outcome(decompose(
        navigation,
        marks,
        field_f64(navigation, "documentBootTotalMs"),
    ))
}

fn outcome(outcome: NavOutcome) -> BootDecompositionOutcome {
    match outcome {
        NavOutcome::NotDecomposed => BootDecompositionOutcome::Incomplete,
        NavOutcome::ClosureViolation => BootDecompositionOutcome::ClosureViolation,
        NavOutcome::Decomposed(_) => BootDecompositionOutcome::Complete,
    }
}

/// A population under accumulation.
#[derive(Default)]
struct Population {
    navigations: usize,
    current: usize,
    mounted: usize,
    style: Vec<StyleSample>,
    bridge: Vec<BridgeSample>,
    direct_complete: usize,
    streaming: usize,
    buffered: usize,
    legacy: usize,
    wasm_api_ms: Vec<f64>,
    wasm_init_ms: Vec<f64>,
    shape_complete: usize,
    shape_missing: usize,
    shapes: BTreeMap<String, usize>,
    closure_violations: usize,
    decomposed: Vec<Decomposition>,
}

/// Sort key putting cold before warm; anything else last, so an unexpected label
/// is visible rather than interleaved.
fn warmth_rank(warmth: &str) -> u8 {
    match warmth {
        "cold" => 0,
        "warm" => 1,
        _ => 2,
    }
}

fn experiment_arm(nav: &Value) -> String {
    nav.get("wasmExperimentArm")
        .and_then(Value::as_str)
        .filter(|arm| !arm.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn shape_signature(nav: &Value) -> Option<String> {
    let shape = nav.get("wasmModuleShape")?.as_object()?;
    let mut parts = Vec::new();
    for key in [
        "imports",
        "importedFunctions",
        "importedTables",
        "importedMemories",
        "exports",
        "exportedFunctions",
        "exportedTables",
        "exportedMemories",
        "customSections",
    ] {
        let value = shape.get(key)?.as_u64()?;
        parts.push(format!("{key}={value}"));
    }
    Some(parts.join(","))
}

/// Group every navigation in `spans` into `(source, project, cacheWarmth, arm)`
/// populations and take the medians.
pub fn boot_phase_rows(spans: &[Span]) -> Result<Vec<BootPhaseRow>> {
    type Key = (String, String, String, String);
    let mut groups: Vec<(Key, Population)> = Vec::new();

    for span in spans.iter().filter(|span| span.name == "e2e.test") {
        let navs = parse_json_attr(&span.raw, "e2e.navigation_top_json", &span.source)?;
        // Both independently optional attributes are validated before either
        // absence can skip the span: a malformed present sibling is never hidden.
        let marks_json = parse_json_attr(&span.raw, "e2e.boot_marks_json", &span.source)?;
        let Some(navs) = navs.as_ref().and_then(Value::as_array) else {
            continue;
        };
        // Boot marks are keyed by navigation id and live on their own attribute:
        // `mount_done.startTime` must come from somewhere other than the segments
        // for the closure check to mean anything.
        let marks_by_nav: BTreeMap<i64, Vec<Value>> = marks_json
            .as_ref()
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        let id = entry.get("id").and_then(Value::as_i64)?;
                        let marks = entry.get("marks").and_then(Value::as_array)?;
                        Some((id, marks.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        for nav in navs {
            let warmth = nav
                .get("cacheWarmth")
                .and_then(Value::as_str)
                .unwrap_or("-")
                .to_string();
            let key: Key = (
                span.source.clone(),
                project_name(span),
                warmth,
                experiment_arm(nav),
            );
            let population = match groups.iter().position(|(k, _)| *k == key) {
                Some(index) => &mut groups[index].1,
                None => {
                    groups.push((key, Population::default()));
                    &mut groups.last_mut().unwrap().1
                }
            };
            population.navigations += 1;
            if field_f64(nav, "commitToMountMs").is_some() {
                population.mounted += 1;
            }
            if let Some(style) = style_sample(nav) {
                population.style.push(style);
            }
            if let Some(bridge) = bridge_sample(nav) {
                population.bridge.push(bridge);
            }
            if nav.get("wasmTimingSchema").and_then(Value::as_str) != Some("direct-init-v1") {
                population.legacy += 1;
                continue;
            }
            population.current += 1;
            let api_ms = field_f64(nav, "wasmApiMs");
            let init_ms = field_f64(nav, "wasmInitMs");
            let path = nav.get("wasmInitPath").and_then(Value::as_str);
            if let (Some(api_ms), Some(init_ms), Some(path @ ("streaming" | "buffered"))) =
                (api_ms, init_ms, path)
                && api_ms >= 0.0
                && init_ms >= 0.0
                && api_ms <= init_ms
            {
                population.direct_complete += 1;
                population.wasm_api_ms.push(api_ms);
                population.wasm_init_ms.push(init_ms);
                match path {
                    "streaming" => population.streaming += 1,
                    "buffered" => population.buffered += 1,
                    _ => unreachable!("path was narrowed above"),
                }
                if let Some(shape) = shape_signature(nav) {
                    population.shape_complete += 1;
                    *population.shapes.entry(shape).or_default() += 1;
                } else {
                    population.shape_missing += 1;
                }
            }

            let marks = nav
                .get("id")
                .and_then(Value::as_i64)
                .and_then(|id| marks_by_nav.get(&id))
                .cloned()
                .unwrap_or_default();
            match decompose(nav, &marks, None) {
                NavOutcome::NotDecomposed => {}
                NavOutcome::ClosureViolation => population.closure_violations += 1,
                NavOutcome::Decomposed(decomposition) => {
                    population.decomposed.push(*decomposition);
                }
            }
        }
    }

    let mut rows: Vec<BootPhaseRow> = groups
        .into_iter()
        .map(
            |((source, project, cache_warmth, experiment_arm), mut population)| {
                let (wasm_api_mean_ms, wasm_api_se_ms) =
                    mean_and_standard_error(&population.wasm_api_ms);
                let (wasm_init_mean_ms, wasm_init_se_ms) =
                    mean_and_standard_error(&population.wasm_init_ms);
                let commit_to_document_start_values: Vec<f64> = population
                    .bridge
                    .iter()
                    .map(|bridge| bridge.commit_to_document_start_ms)
                    .collect();
                let mount_done_to_binding_values: Vec<f64> = population
                    .bridge
                    .iter()
                    .map(|bridge| bridge.mount_done_to_binding_ms)
                    .collect();
                let frame_skew_remainder_values: Vec<f64> = population
                    .bridge
                    .iter()
                    .map(|bridge| bridge.frame_skew_remainder_ms)
                    .collect();
                let (commit_to_document_start_mean_ms, commit_to_document_start_se_ms) =
                    mean_and_standard_error(&commit_to_document_start_values);
                let (mount_done_to_binding_mean_ms, mount_done_to_binding_se_ms) =
                    mean_and_standard_error(&mount_done_to_binding_values);
                let (frame_skew_remainder_mean_ms, frame_skew_remainder_se_ms) =
                    mean_and_standard_error(&frame_skew_remainder_values);
                let style_max_response_end_values: Vec<f64> = population
                    .style
                    .iter()
                    .map(|style| style.style_max_response_end_ms)
                    .collect();
                let style_to_module_before_init_values: Vec<f64> = population
                    .style
                    .iter()
                    .map(|style| style.style_to_module_before_init_ms)
                    .collect();
                let module_before_init_to_wasm_fetch_start_values: Vec<f64> = population
                    .style
                    .iter()
                    .map(|style| style.module_before_init_to_wasm_fetch_start_ms)
                    .collect();
                let wasm_fetch_start_values: Vec<f64> = population
                    .style
                    .iter()
                    .map(|style| style.wasm_fetch_start_ms)
                    .collect();
                let (style_max_response_end_mean_ms, style_max_response_end_se_ms) =
                    mean_and_standard_error(&style_max_response_end_values);
                let (style_to_module_before_init_mean_ms, style_to_module_before_init_se_ms) =
                    mean_and_standard_error(&style_to_module_before_init_values);
                let (
                    module_before_init_to_wasm_fetch_start_mean_ms,
                    module_before_init_to_wasm_fetch_start_se_ms,
                ) = mean_and_standard_error(&module_before_init_to_wasm_fetch_start_values);
                let (wasm_fetch_start_mean_ms, wasm_fetch_start_se_ms) =
                    mean_and_standard_error(&wasm_fetch_start_values);
                let style_complete = population.style.len();
                let style_ordering_pass = population
                    .style
                    .iter()
                    .filter(|style| style.style_to_module_before_init_ms >= 0.0)
                    .count();
                let style_decisive =
                    population.navigations > 0 && style_complete == population.navigations;
                let frame_skew_remainder_max_abs_ms = frame_skew_remainder_values
                    .iter()
                    .map(|value| value.abs())
                    .max_by(|left, right| {
                        left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
                    });
                let bridge_complete = population.bridge.len();
                let bridge_decisive = population.mounted > 0
                    && bridge_complete == population.mounted
                    && frame_skew_remainder_mean_ms
                        .is_some_and(|mean| mean.abs() <= BRIDGE_MEAN_TOLERANCE_MS)
                    && frame_skew_remainder_max_abs_ms
                        .is_some_and(|max| max <= BRIDGE_MAX_TOLERANCE_MS);
                BootPhaseRow {
                    source,
                    project,
                    cache_warmth,
                    experiment_arm,
                    navigations: population.navigations,
                    decomposed: population.decomposed.len(),
                    current: population.current,
                    mounted: population.mounted,
                    bridge_complete,
                    bridge_decisive,
                    style_complete,
                    style_decisive,
                    style_ordering_pass,
                    style_to_module_before_init_mean_ms,
                    style_to_module_before_init_se_ms,
                    module_before_init_to_wasm_fetch_start_mean_ms,
                    module_before_init_to_wasm_fetch_start_se_ms,
                    style_max_response_end_mean_ms,
                    style_max_response_end_se_ms,
                    wasm_fetch_start_mean_ms,
                    wasm_fetch_start_se_ms,
                    direct_complete: population.direct_complete,
                    direct_missing: population.current - population.direct_complete,
                    streaming: population.streaming,
                    buffered: population.buffered,
                    legacy: population.legacy,
                    wasm_api_ms: median(&mut population.wasm_api_ms),
                    wasm_init_ms: median(&mut population.wasm_init_ms),
                    shape_complete: population.shape_complete,
                    shape_missing: population.shape_missing,
                    wasm_api_mean_ms,
                    wasm_api_se_ms,
                    wasm_init_mean_ms,
                    wasm_init_se_ms,
                    commit_to_document_start_mean_ms,
                    commit_to_document_start_se_ms,
                    mount_done_to_binding_mean_ms,
                    mount_done_to_binding_se_ms,
                    frame_skew_remainder_mean_ms,
                    frame_skew_remainder_se_ms,
                    frame_skew_remainder_max_abs_ms,
                    unique_shapes: population.shapes.len(),
                    closure_violations: population.closure_violations,
                    segments: segment_medians(&population.decomposed),
                    boot_total_ms: median(
                        &mut population
                            .decomposed
                            .iter()
                            .map(|d| d.boot_total_ms)
                            .collect::<Vec<_>>(),
                    ),
                    commit_to_mount_ms: median(
                        &mut population
                            .decomposed
                            .iter()
                            .filter_map(|d| d.commit_to_mount_ms)
                            .collect::<Vec<_>>(),
                    ),
                    frame_skew_ms: median(
                        &mut population
                            .decomposed
                            .iter()
                            .filter_map(|d| Some(d.commit_to_mount_ms? - d.boot_total_ms))
                            .collect::<Vec<_>>(),
                    ),
                }
            },
        )
        .collect();
    rows.sort_by(|left, right| {
        (
            &left.source,
            &left.project,
            warmth_rank(&left.cache_warmth),
            &left.cache_warmth,
            &left.experiment_arm,
        )
            .cmp(&(
                &right.source,
                &right.project,
                warmth_rank(&right.cache_warmth),
                &right.cache_warmth,
                &right.experiment_arm,
            ))
    });
    Ok(rows)
}

/// The span's `e2e.project`, or `-` when unset (matching `analyze`'s label).
fn project_name(span: &Span) -> String {
    if span.project.is_empty() {
        "-".to_string()
    } else {
        span.project.clone()
    }
}

/// Per-label medians in first-seen segment order.
fn segment_medians(decompositions: &[Decomposition]) -> Vec<SegmentMedian> {
    let mut groups: Vec<(String, Vec<f64>)> = Vec::new();
    for decomposition in decompositions {
        for (label, value) in &decomposition.segments {
            match groups.iter().position(|(known, _)| known == label) {
                Some(index) => groups[index].1.push(*value),
                None => groups.push((label.clone(), vec![*value])),
            }
        }
    }
    groups
        .into_iter()
        .filter_map(|(label, mut values)| {
            let count = values.len();
            Some(SegmentMedian {
                label,
                median_ms: median(&mut values)?,
                count,
            })
        })
        .collect()
}

#[derive(Tabled)]
struct SegmentDisplay {
    segment: String,
    median_ms: String,
    #[tabled(rename = "share_of_boot_total_%")]
    share: String,
    n: usize,
}
fn bridge_status(row: &BootPhaseRow) -> String {
    if row.bridge_decisive {
        return "decisive".to_string();
    }
    let mut reasons = Vec::new();
    if row.mounted == 0 {
        reasons.push("no mounted navigations");
    }
    if row.bridge_complete != row.mounted {
        reasons.push("coverage incomplete");
    } else {
        if row
            .frame_skew_remainder_mean_ms
            .is_none_or(|mean| mean.abs() > BRIDGE_MEAN_TOLERANCE_MS)
        {
            reasons.push("mean remainder out of bounds");
        }
        if row
            .frame_skew_remainder_max_abs_ms
            .is_none_or(|max| max > BRIDGE_MAX_TOLERANCE_MS)
        {
            reasons.push("max remainder out of bounds");
        }
    }
    format!("non-decisive ({})", reasons.join(", "))
}

fn style_status(row: &BootPhaseRow) -> String {
    if row.style_decisive {
        return "decisive".to_string();
    }
    let mut reasons = Vec::new();
    if row.navigations == 0 {
        reasons.push("no navigations");
    }
    if row.style_complete != row.navigations {
        reasons.push("coverage incomplete");
    }
    format!("non-decisive ({})", reasons.join(", "))
}

/// Render the rows as one block per population.
pub fn render(rows: &[BootPhaseRow]) -> String {
    if rows.is_empty() {
        return "No navigations found in the provided trace files.\n".to_string();
    }
    let mut out = String::from(
        "Boot-phase decomposition (medians; even counts take the lower middle value).\n\
         Segments are document-relative and close on mount_done.startTime exactly;\n\
         commitToMountMs is Node-side wall clock and is reported, never decomposed.\n\
         Frame-skew bridge diagnostics stay outside the app boot decomposition table.\n\
         Per-segment medians are marginal, so they need not sum to the median total.\n\n",
    );
    for row in rows {
        out.push_str(&format!(
            "== {} / {} / {} / arm:{} ==\n",
            row.source, row.project, row.cache_warmth, row.experiment_arm
        ));
        out.push_str(&format!(
            "navigations: {}  decomposed: {}  closure violations: {}\n",
            row.navigations, row.decomposed, row.closure_violations
        ));
        let style_ordering_rate = if row.style_complete > 0 {
            Some(row.style_ordering_pass as f64 / row.style_complete as f64 * 100.0)
        } else {
            None
        };
        out.push_str(&format!(
            "current: {}  mounted: {}  bridge complete: {}  bridge certification: {}\n\
             stylesheet complete: {}  stylesheet certification: {}\n\
             stylesheet ordering pass (styleToModuleBeforeInitMs >= 0): {}/{} ({})\n\
             mean styleToModuleBeforeInitMs ± SE (n={}): {} ± {}\n\
             mean moduleBeforeInitToWasmFetchStartMs ± SE (n={}): {} ± {}\n\
             mean styleMaxResponseEndMs ± SE (n={}): {} ± {}\n\
             mean wasmFetchStartMs ± SE (n={}): {} ± {}\n\
             direct complete: {}  direct missing: {}  streaming: {}  buffered: {}  legacy: {}\n\
             shape complete: {}  shape missing: {}  unique shapes: {}\n\
             median wasmApiMs (n={}): {}  median wasmInitMs (n={}): {}\n\
             mean wasmApiMs ± SE: {} ± {}  mean wasmInitMs ± SE: {} ± {}\n\
             mean commitToDocumentStartMs ± SE (n={}): {} ± {}\n\
             mean mountDoneToBindingMs ± SE (n={}): {} ± {}\n\
             mean frameSkewRemainderMs ± SE (n={}): {} ± {}  max |remainder|: {}\n",
            row.current,
            row.mounted,
            row.bridge_complete,
            bridge_status(row),
            row.style_complete,
            style_status(row),
            row.style_ordering_pass,
            row.style_complete,
            style_ordering_rate.map_or_else(|| "-".to_string(), |rate| format!("{rate:.1}%")),
            row.style_complete,
            optional_ms(row.style_to_module_before_init_mean_ms),
            optional_ms(row.style_to_module_before_init_se_ms),
            row.style_complete,
            optional_ms(row.module_before_init_to_wasm_fetch_start_mean_ms),
            optional_ms(row.module_before_init_to_wasm_fetch_start_se_ms),
            row.style_complete,
            optional_ms(row.style_max_response_end_mean_ms),
            optional_ms(row.style_max_response_end_se_ms),
            row.style_complete,
            optional_ms(row.wasm_fetch_start_mean_ms),
            optional_ms(row.wasm_fetch_start_se_ms),
            row.direct_complete,
            row.direct_missing,
            row.streaming,
            row.buffered,
            row.legacy,
            row.shape_complete,
            row.shape_missing,
            row.unique_shapes,
            row.direct_complete,
            optional_ms(row.wasm_api_ms),
            row.direct_complete,
            optional_ms(row.wasm_init_ms),
            optional_ms(row.wasm_api_mean_ms),
            optional_ms(row.wasm_api_se_ms),
            optional_ms(row.wasm_init_mean_ms),
            optional_ms(row.wasm_init_se_ms),
            row.bridge_complete,
            optional_ms(row.commit_to_document_start_mean_ms),
            optional_ms(row.commit_to_document_start_se_ms),
            row.bridge_complete,
            optional_ms(row.mount_done_to_binding_mean_ms),
            optional_ms(row.mount_done_to_binding_se_ms),
            row.bridge_complete,
            optional_ms(row.frame_skew_remainder_mean_ms),
            optional_ms(row.frame_skew_remainder_se_ms),
            optional_ms(row.frame_skew_remainder_max_abs_ms),
        ));
        // Loud, because an empty table and "the instrument was dark" look
        // identical otherwise — and that is the #818 failure mode itself.
        if row.decomposed == 0 {
            out.push_str(&format!(
                "no decomposed navigations (0 of {}) — nothing to decompose for this population\n\n",
                row.navigations
            ));
            continue;
        }
        out.push_str(&format!(
            "median bootTotalMs: {}\n",
            optional_ms(row.boot_total_ms)
        ));
        out.push_str(&format!(
            "median commitToMountMs: {}\n",
            optional_ms(row.commit_to_mount_ms)
        ));
        out.push_str(&format!(
            "median frame skew (commitToMountMs - bootTotalMs): {}\n",
            optional_ms(row.frame_skew_ms)
        ));
        let total = row.boot_total_ms.unwrap_or(0.0);
        let display: Vec<SegmentDisplay> = row
            .segments
            .iter()
            .map(|segment| SegmentDisplay {
                segment: segment.label.clone(),
                median_ms: format!("{:.3}", segment.median_ms),
                share: if total > 0.0 {
                    format!("{:.1}", segment.median_ms / total * 100.0)
                } else {
                    "-".to_string()
                },
                n: segment.count,
            })
            .collect();
        out.push_str(&Table::new(display).with(Style::sharp()).to_string());
        out.push_str("\n\n");
    }
    out
}

fn optional_ms(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:.3}"))
}

/// Read every input and report the medians.
pub fn boot_phases(inputs: &[PathBuf]) -> Result<Vec<BootPhaseRow>> {
    let mut spans = Vec::new();
    for input in inputs {
        spans.extend(read_spans(input, &Filters::default())?);
    }
    boot_phase_rows(&spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::parse::parse_spans;
    use serde_json::json;

    /// The canonical timeline: initialization starts at 10, then reaches
    /// `boot.entry` at 35; Rust phases then reach `mount_done`.
    const INIT_START: f64 = 10.0;
    const ENTRY: f64 = 35.0;
    const SEED_PARSED: f64 = 50.0;
    const RENDER_START: f64 = 75.0;
    fn shape(imports: u64, exports: u64) -> Value {
        json!({
            "imports": imports,
            "importedFunctions": imports,
            "importedTables": 0,
            "importedMemories": 0,
            "exports": exports,
            "exportedFunctions": exports,
            "exportedTables": 0,
            "exportedMemories": 1,
            "customSections": 0,
        })
    }

    fn mark(name: &str, start_time: f64) -> Value {
        json!({ "name": name, "startTime": start_time })
    }

    fn boot_marks(mount_done_ms: f64) -> Vec<Value> {
        vec![
            mark("jaunder.boot.entry", ENTRY),
            mark("jaunder.boot.seed_parsed", SEED_PARSED),
            mark("jaunder.boot.render_start", RENDER_START),
            mark("jaunder.boot.mount_done", mount_done_ms),
        ]
    }

    fn boot_phases_object(mount_done_ms: f64) -> Value {
        json!({
            "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
            "jaunder.boot.seed_parsed->jaunder.boot.render_start": RENDER_START - SEED_PARSED,
            "jaunder.boot.render_start->jaunder.boot.mount_done": mount_done_ms - RENDER_START,
        })
    }

    fn nav(id: i64, warmth: &str, mount_done_ms: f64, commit_to_mount_ms: Option<f64>) -> Value {
        json!({
            "id": id,
            "cacheWarmth": warmth,
            "wasmTimingSchema": "direct-init-v1",
            "commitToMountMs": commit_to_mount_ms,
            "wasmInitStartMs": INIT_START,
            "wasmInitStartToBootEntryMs": ENTRY - INIT_START,
            "bootPhases": boot_phases_object(mount_done_ms),
        })
    }

    fn add_bridge(
        nav: &mut Value,
        document_time_origin_ms: f64,
        document_boot_total_ms: f64,
        commit_to_document_start_ms: f64,
        mount_done_to_binding_ms: f64,
        frame_skew_remainder_ms: f64,
    ) {
        let object = nav.as_object_mut().unwrap();
        object.insert("frameSkewSchema".to_string(), json!(BRIDGE_SCHEMA));
        object.insert(
            "documentTimeOriginMs".to_string(),
            json!(document_time_origin_ms),
        );
        object.insert(
            "documentBootTotalMs".to_string(),
            json!(document_boot_total_ms),
        );
        object.insert(
            "commitToDocumentStartMs".to_string(),
            json!(commit_to_document_start_ms),
        );
        object.insert(
            "mountDoneToBindingMs".to_string(),
            json!(mount_done_to_binding_ms),
        );
        object.insert(
            "frameSkewRemainderMs".to_string(),
            json!(frame_skew_remainder_ms),
        );
    }

    fn add_style(
        nav: &mut Value,
        module_before_init_ms: f64,
        jaunder_css_response_end_ms: f64,
        jaunder_themes_css_response_end_ms: f64,
        wasm_fetch_start_ms: f64,
    ) {
        let style_max_response_end_ms =
            jaunder_css_response_end_ms.max(jaunder_themes_css_response_end_ms);
        let object = nav.as_object_mut().unwrap();
        object.insert(
            "moduleBeforeInitMs".to_string(),
            json!(module_before_init_ms),
        );
        object.insert(
            "jaunderCssResponseEndMs".to_string(),
            json!(jaunder_css_response_end_ms),
        );
        object.insert(
            "jaunderThemesCssResponseEndMs".to_string(),
            json!(jaunder_themes_css_response_end_ms),
        );
        object.insert(
            "styleMaxResponseEndMs".to_string(),
            json!(style_max_response_end_ms),
        );
        object.insert(
            "styleToModuleBeforeInitMs".to_string(),
            json!(module_before_init_ms - style_max_response_end_ms),
        );
        object.insert(
            "moduleBeforeInitToWasmFetchStartMs".to_string(),
            json!(wasm_fetch_start_ms - module_before_init_ms),
        );
        object.insert("wasmFetchStartMs".to_string(), json!(wasm_fetch_start_ms));
    }

    /// An unversioned navigation with no boot marks — retained raw legacy input.
    fn dark_nav(id: i64, warmth: &str) -> Value {
        json!({
            "id": id,
            "cacheWarmth": warmth,
            "commitToMountMs": 300.0,
            "bootPhases": Value::Null,
        })
    }

    fn attr(key: &str, value: Value) -> Value {
        json!({ "key": key, "value": value })
    }

    /// One `e2e.test` span carrying `navs` and their marks, parsed out of the trace
    /// file `source`.
    fn test_span(
        source: &str,
        project: &str,
        navs: Vec<Value>,
        marks: Vec<(i64, Vec<Value>)>,
    ) -> Vec<Span> {
        let marks_json = Value::Array(
            marks
                .into_iter()
                .map(|(id, marks)| json!({ "id": id, "marks": marks }))
                .collect(),
        );
        let line = json!({
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [{
                        "name": "e2e.test",
                        "attributes": [
                            attr("e2e.project", json!({ "stringValue": project })),
                            attr(
                                "e2e.navigation_top_json",
                                json!({ "stringValue": Value::Array(navs).to_string() }),
                            ),
                            attr(
                                "e2e.boot_marks_json",
                                json!({ "stringValue": marks_json.to_string() }),
                            ),
                        ],
                    }]
                }]
            }]
        });
        parse_spans(&line.to_string(), &Filters::default(), source).unwrap()
    }

    /// One warm, fully decomposed navigation ending at `mount_done_ms`.
    fn one_nav_span(source: &str, project: &str, mount_done_ms: f64) -> Vec<Span> {
        test_span(
            source,
            project,
            vec![nav(1, "warm", mount_done_ms, Some(mount_done_ms + 50.0))],
            vec![(1, boot_marks(mount_done_ms))],
        )
    }

    #[test]
    fn boot_phase_segments_sum_to_the_boot_total_within_a_millisecond() {
        // The exclusive pre-boot segments plus Rust intervals close to
        // `mount_done.startTime`.
        let rows = boot_phase_rows(&one_nav_span("sqlite", "chromium", 105.0)).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            (row.navigations, row.decomposed, row.closure_violations),
            (1, 1, 0)
        );
        assert_eq!(
            row.segments.len(),
            5,
            "two exclusive segments + three intervals"
        );
        let sum: f64 = row.segments.iter().map(|s| s.median_ms).sum();
        assert_eq!(row.boot_total_ms, Some(105.0));
        assert!(
            (sum - 105.0).abs() <= CLOSURE_TOLERANCE_MS,
            "segments summed to {sum}, not the boot total 105",
        );
    }

    #[test]
    fn boot_phase_reports_commit_to_mount_and_frame_skew_without_decomposing_them() {
        // `commitToMountMs` is `Date.now()`-derived and every segment is
        // `timeOrigin`-derived; the skew is real, bidirectional harness overhead
        // and gets its own figure rather than a segment (spec D8, AC14).
        let rows = boot_phase_rows(&one_nav_span("sqlite", "chromium", 105.0)).unwrap();
        assert_eq!(rows[0].commit_to_mount_ms, Some(155.0));
        assert_eq!(rows[0].frame_skew_ms, Some(50.0));
        assert!(
            !rows[0]
                .segments
                .iter()
                .any(|s| s.label.contains("commit") || s.label.contains("skew")),
            "the wall-clock quantities must never appear as segments",
        );
    }

    #[test]
    fn boot_phase_bridge_rows_report_complete_decisive_coverage() {
        let mut first = nav(1, "warm", 105.0, Some(155.0));
        add_bridge(&mut first, 1_000.0, 105.0, 15.0, 35.0, 0.5);
        let mut second = nav(2, "warm", 95.0, Some(145.0));
        add_bridge(&mut second, 1_100.0, 95.0, 18.0, 31.0, -0.5);
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![first, second],
            vec![(1, boot_marks(105.0)), (2, boot_marks(95.0))],
        ))
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.mounted, 2);
        assert_eq!(row.bridge_complete, 2);
        assert!(row.bridge_decisive);
        assert_eq!(row.commit_to_document_start_mean_ms, Some(16.5));
        assert_eq!(row.mount_done_to_binding_mean_ms, Some(33.0));
        assert_eq!(row.frame_skew_remainder_mean_ms, Some(0.0));
        assert_eq!(row.frame_skew_remainder_max_abs_ms, Some(0.5));
    }

    #[test]
    fn boot_phase_bridge_rows_mark_incomplete_coverage_non_decisive() {
        let mut complete = nav(1, "warm", 105.0, Some(155.0));
        add_bridge(&mut complete, 1_000.0, 105.0, 15.0, 35.0, 0.0);
        let missing = nav(2, "warm", 95.0, Some(145.0));
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![complete, missing],
            vec![(1, boot_marks(105.0)), (2, boot_marks(95.0))],
        ))
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.mounted, 2);
        assert_eq!(row.bridge_complete, 1);
        assert!(!row.bridge_decisive);
        assert!(render(&rows).contains("bridge certification: non-decisive (coverage incomplete"));
    }

    #[test]
    fn boot_phase_bridge_rows_mark_closure_failure_non_decisive() {
        let mut navigation = nav(1, "warm", 105.0, Some(155.0));
        add_bridge(&mut navigation, 1_000.0, 105.0, 15.0, 35.0, 2.5);
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![navigation],
            vec![(1, boot_marks(105.0))],
        ))
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.bridge_complete, 1);
        assert_eq!(row.frame_skew_remainder_mean_ms, Some(2.5));
        assert_eq!(row.frame_skew_remainder_max_abs_ms, Some(2.5));
        assert!(!row.bridge_decisive);
        let out = render(&rows);
        assert!(out.contains("mean frameSkewRemainderMs ± SE (n=1): 2.500 ± -"));
        assert!(out.contains("max |remainder|: 2.500"));
    }

    #[test]
    fn boot_phase_bridge_rows_treat_historical_missing_fields_as_non_decisive_not_panic() {
        let historical = nav(1, "warm", 105.0, Some(155.0));
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![historical],
            vec![(1, boot_marks(105.0))],
        ))
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.mounted, 1);
        assert_eq!(row.bridge_complete, 0);
        assert!(!row.bridge_decisive);
        assert!(row.commit_to_document_start_mean_ms.is_none());
        assert!(row.mount_done_to_binding_mean_ms.is_none());
        assert!(row.frame_skew_remainder_mean_ms.is_none());
    }

    #[test]
    fn stylesheet_diagnostics_report_complete_coverage_and_ordering_rate() {
        let mut first = nav(1, "warm", 105.0, Some(155.0));
        add_style(&mut first, 120.0, 80.0, 90.0, 150.0);
        let mut second = nav(2, "warm", 95.0, Some(145.0));
        add_style(&mut second, 95.0, 100.0, 90.0, 140.0);
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![first, second],
            vec![(1, boot_marks(105.0)), (2, boot_marks(95.0))],
        ))
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.style_complete, 2);
        assert!(row.style_decisive);
        assert_eq!(row.style_ordering_pass, 1);
        assert_eq!(row.style_to_module_before_init_mean_ms, Some(12.5));
        assert_eq!(
            row.module_before_init_to_wasm_fetch_start_mean_ms,
            Some(37.5)
        );
        assert_eq!(row.style_max_response_end_mean_ms, Some(95.0));
        assert_eq!(row.wasm_fetch_start_mean_ms, Some(145.0));
        let report = render(&rows);
        assert!(
            report.contains("stylesheet certification: decisive"),
            "{report}"
        );
        assert!(
            report
                .contains("stylesheet ordering pass (styleToModuleBeforeInitMs >= 0): 1/2 (50.0%)"),
            "{report}"
        );
    }

    #[test]
    fn stylesheet_diagnostics_mark_historical_missing_rows_non_decisive_without_panic() {
        let mut complete = nav(1, "warm", 105.0, Some(155.0));
        add_style(&mut complete, 120.0, 80.0, 90.0, 150.0);
        let historical = nav(2, "warm", 95.0, Some(145.0));
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![complete, historical],
            vec![(1, boot_marks(105.0)), (2, boot_marks(95.0))],
        ))
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.style_complete, 1);
        assert!(!row.style_decisive);
        assert_eq!(row.style_ordering_pass, 1);
        assert_eq!(row.style_to_module_before_init_mean_ms, Some(30.0));
        let report = render(&rows);
        assert!(
            report.contains("stylesheet certification: non-decisive (coverage incomplete)"),
            "{report}"
        );
    }

    #[test]
    fn stylesheet_diagnostics_report_ordering_failures_decisively() {
        let mut navigation = nav(1, "warm", 105.0, Some(155.0));
        add_style(&mut navigation, 95.0, 100.0, 90.0, 140.0);
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![navigation],
            vec![(1, boot_marks(105.0))],
        ))
        .unwrap();
        let row = &rows[0];
        assert!(row.style_decisive);
        assert_eq!(row.style_ordering_pass, 0);
        assert_eq!(row.style_to_module_before_init_mean_ms, Some(-5.0));
        let report = render(&rows);
        assert!(
            report
                .contains("stylesheet ordering pass (styleToModuleBeforeInitMs >= 0): 0/1 (0.0%)"),
            "{report}"
        );
    }

    #[test]
    fn stylesheet_diagnostics_render_outside_the_decomposition_table() {
        let mut navigation = dark_nav(1, "warm");
        add_style(&mut navigation, 120.0, 80.0, 90.0, 150.0);
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![navigation],
            vec![(1, Vec::new())],
        ))
        .unwrap();
        assert_eq!(rows[0].decomposed, 0);
        let report = render(&rows);
        let stylesheet_index = report.find("stylesheet certification: decisive").unwrap();
        let no_decomposition_index = report.find("no decomposed navigations").unwrap();
        assert!(stylesheet_index < no_decomposition_index, "{report}");
    }

    #[test]
    fn a_boot_phase_navigation_failing_closure_is_counted_not_silently_included() {
        // `mount_done` at 200 while the segments describe 105: the segments do not
        // describe this navigation, so it is a named violation, not a sample.
        let spans = test_span(
            "sqlite",
            "chromium",
            vec![nav(1, "warm", 105.0, Some(160.0))],
            vec![(1, boot_marks(200.0))],
        );
        let rows = boot_phase_rows(&spans).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].navigations, 1);
        assert_eq!(rows[0].closure_violations, 1);
        assert_eq!(
            rows[0].decomposed, 0,
            "a violation is excluded from the medians"
        );
    }

    #[test]
    fn navigation_document_total_closes_page_evidence_without_boot_marks() {
        let mut navigation = nav(1, "warm", 105.0, Some(160.0));
        navigation["documentBootTotalMs"] = json!(105.0);
        assert!(matches!(
            page_boot_decomposition_outcome(&navigation, &[]),
            BootDecompositionOutcome::Complete
        ));

        navigation["documentBootTotalMs"] = json!(107.0);
        assert!(matches!(
            page_boot_decomposition_outcome(&navigation, &[]),
            BootDecompositionOutcome::ClosureViolation
        ));
    }

    #[test]
    fn boot_phase_rows_split_cold_from_warm() {
        // Spec D6: cold pays the whole wasm download and warm does not; pooling
        // them describes neither.
        let spans = test_span(
            "sqlite",
            "chromium",
            vec![
                nav(1, "cold", 105.0, Some(155.0)),
                nav(2, "warm", 95.0, Some(145.0)),
            ],
            vec![(1, boot_marks(105.0)), (2, boot_marks(95.0))],
        );
        let rows = boot_phase_rows(&spans).unwrap();
        assert_eq!(rows.len(), 2, "cold and warm are never pooled");
        assert_eq!(rows[0].cache_warmth, "cold", "cold sorts first");
        assert_eq!(rows[0].boot_total_ms, Some(105.0));
        assert_eq!(rows[1].cache_warmth, "warm");
        assert_eq!(rows[1].boot_total_ms, Some(95.0));
    }

    #[test]
    fn boot_phase_rows_split_by_source_so_backends_never_pool() {
        // `projectName` is the browser and names no backend
        // (`traces/run.rs:99-101`), so on project alone sqlite pools with postgres.
        let mut spans = one_nav_span("sqlite", "firefox", 105.0);
        spans.extend(one_nav_span("postgres", "firefox", 205.0));
        let rows = boot_phase_rows(&spans).unwrap();
        assert_eq!(rows.len(), 2, "sqlite and postgres must not be pooled");
        assert!(rows.iter().all(|r| r.project == "firefox"));
        assert!(rows.iter().any(|r| r.source == "sqlite"));
        assert!(rows.iter().any(|r| r.source == "postgres"));
    }

    #[test]
    fn boot_phase_intervals_are_selected_by_key_so_a_fourth_mark_extends_the_table() {
        // A new mark in `client::perf` yields a FOURTH interval. Selecting by the
        // `boot.` key substring rather than by position or count keeps closure —
        // pinning three would read a richer capture as a broken one.
        let marks = vec![
            mark("jaunder.boot.entry", ENTRY),
            mark("jaunder.boot.seed_parsed", SEED_PARSED),
            mark("jaunder.boot.render_start", RENDER_START),
            mark("jaunder.boot.hydrated", 90.0),
            mark("jaunder.boot.mount_done", 105.0),
        ];
        let navigation = json!({
            "id": 1,
            "cacheWarmth": "warm",
            "wasmTimingSchema": "direct-init-v1",
            "commitToMountMs": 155.0,
            "wasmInitStartMs": INIT_START,
            "wasmInitStartToBootEntryMs": ENTRY - INIT_START,
            "bootPhases": {
                "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
                "jaunder.boot.seed_parsed->jaunder.boot.render_start": RENDER_START - SEED_PARSED,
                "jaunder.boot.render_start->jaunder.boot.hydrated": 90.0 - RENDER_START,
                "jaunder.boot.hydrated->jaunder.boot.mount_done": 105.0 - 90.0,
            },
        });
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![navigation],
            vec![(1, marks)],
        ))
        .unwrap();
        assert_eq!(
            rows[0].decomposed, 1,
            "a fourth mark must not break closure"
        );
        assert_eq!(rows[0].closure_violations, 0);
        assert_eq!(
            rows[0].segments.len(),
            6,
            "two exclusive segments + four intervals"
        );
        // And in `startTime` order, not the object's key order.
        let labels: Vec<&str> = rows[0].segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels[2], "jaunder.boot.entry->jaunder.boot.seed_parsed");
        assert_eq!(labels[5], "jaunder.boot.hydrated->jaunder.boot.mount_done");
    }

    #[test]
    fn boot_phase_segments_stay_in_timeline_order_when_a_phase_is_zero_length() {
        // Observed on the real corpus: sub-millisecond phases report as 0, so two
        // intervals share a `from` startTime and the JSON object's ALPHABETICAL key
        // order decided the sequence — `render_start->mount_done` printed before
        // `seed_parsed->render_start`.
        let marks = vec![
            mark("jaunder.boot.entry", ENTRY),
            mark("jaunder.boot.seed_parsed", SEED_PARSED),
            mark("jaunder.boot.render_start", SEED_PARSED),
            mark("jaunder.boot.mount_done", 105.0),
        ];
        let navigation = json!({
            "id": 1,
            "cacheWarmth": "warm",
            "wasmTimingSchema": "direct-init-v1",
            "commitToMountMs": 155.0,
            "wasmInitStartMs": INIT_START,
            "wasmInitStartToBootEntryMs": ENTRY - INIT_START,
            "bootPhases": {
                "jaunder.boot.entry->jaunder.boot.seed_parsed": SEED_PARSED - ENTRY,
                "jaunder.boot.seed_parsed->jaunder.boot.render_start": 0.0,
                "jaunder.boot.render_start->jaunder.boot.mount_done": 105.0 - SEED_PARSED,
            },
        });
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![navigation],
            vec![(1, marks)],
        ))
        .unwrap();
        let labels: Vec<&str> = rows[0].segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels[3],
            "jaunder.boot.seed_parsed->jaunder.boot.render_start"
        );
        assert_eq!(
            labels[4],
            "jaunder.boot.render_start->jaunder.boot.mount_done"
        );
    }

    #[test]
    fn boot_phase_medians_are_the_lower_of_the_two_middle_values_on_even_counts() {
        // Pin the convention: `traces analyze` computes no medians, so there is
        // none to inherit.
        let totals = [100.0, 200.0, 300.0, 400.0];
        let navs: Vec<Value> = totals
            .iter()
            .enumerate()
            .map(|(index, total)| nav(index as i64 + 1, "warm", *total, Some(total + 10.0)))
            .collect();
        let marks: Vec<(i64, Vec<Value>)> = totals
            .iter()
            .enumerate()
            .map(|(index, total)| (index as i64 + 1, boot_marks(*total)))
            .collect();
        let rows = boot_phase_rows(&test_span("sqlite", "chromium", navs, marks)).unwrap();
        assert_eq!(rows[0].decomposed, 4);
        assert_eq!(
            rows[0].boot_total_ms,
            Some(200.0),
            "the lower middle of 200/300, not their mean",
        );
        let tail = rows[0].segments.last().unwrap();
        assert_eq!(tail.median_ms, 200.0 - RENDER_START);
        assert_eq!(tail.count, 4);
    }

    #[test]
    fn a_population_with_no_decomposed_navigations_reports_that_explicitly() {
        // The #818 failure mode: firefox pre-fix. Not an empty table, not a
        // division by zero — a loud line.
        let spans = test_span(
            "sqlite",
            "firefox",
            vec![dark_nav(1, "cold"), dark_nav(2, "warm")],
            vec![(1, vec![]), (2, vec![])],
        );
        let rows = boot_phase_rows(&spans).unwrap();
        assert_eq!(rows.len(), 2, "the population is still reported");
        assert!(rows.iter().all(|r| r.decomposed == 0));
        assert!(rows.iter().all(|r| r.boot_total_ms.is_none()));
        assert!(rows.iter().all(|r| r.segments.is_empty()));
        let out = render(&rows);
        assert!(
            out.contains("no decomposed navigations"),
            "the blackout must be stated, not implied by an absent table:\n{out}",
        );
    }

    #[test]
    fn boot_phase_render_names_every_population_and_its_violations() {
        let mut navigation = nav(1, "warm", 105.0, Some(155.0));
        add_bridge(&mut navigation, 1_000.0, 105.0, 15.0, 35.0, 0.0);
        let mut spans = test_span(
            "sqlite",
            "chromium",
            vec![navigation],
            vec![(1, boot_marks(105.0))],
        );
        spans.extend(one_nav_span("postgres", "firefox", 205.0));
        let out = render(&boot_phase_rows(&spans).unwrap());
        assert!(out.contains("sqlite / chromium / warm"));
        assert!(out.contains("postgres / firefox / warm"));
        assert!(out.contains("closure violations: 0"));
        assert!(out.contains("bridge certification: decisive"));
        assert!(out.contains("mean commitToDocumentStartMs ± SE"));
        assert!(out.contains("median frame skew"));
    }

    #[test]
    fn boot_phase_render_of_nothing_says_so() {
        assert!(render(&[]).contains("No navigations"));
    }

    #[test]
    fn boot_phases_reads_files() {
        let dir = std::env::temp_dir().join(format!("traces-boot-phases-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("otel-traces.jsonl");
        let spans = one_nav_span("inline", "chromium", 105.0);
        let line = json!({
            "resourceSpans": [{ "scopeSpans": [{ "spans": [spans[0].raw.clone()] }] }]
        });
        std::fs::write(&file, format!("{line}\n")).unwrap();

        let rows = boot_phases(&[file]).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].decomposed, 1);
    }

    #[test]
    fn trace_json_attr_boot_phases_fail_on_malformed_present_value() {
        let dir =
            std::env::temp_dir().join(format!("traces-boot-phases-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("otel-traces.jsonl");
        let mut raw = one_nav_span("inline", "chromium", 105.0)[0].raw.clone();
        let attributes = raw
            .get_mut("attributes")
            .and_then(Value::as_array_mut)
            .unwrap();
        let navigation = attributes
            .iter_mut()
            .find(|attr| attr.get("key").and_then(Value::as_str) == Some("e2e.navigation_top_json"))
            .unwrap();
        navigation["value"]["stringValue"] = Value::String("{not json".to_owned());
        let line = json!({
            "resourceSpans": [{ "scopeSpans": [{ "spans": [raw] }] }]
        });
        std::fs::write(&file, format!("{line}\n")).unwrap();

        let error = boot_phases(&[file]).unwrap_err();
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

    #[test]
    fn trace_json_attr_absent_navigation_does_not_hide_malformed_boot_marks() {
        let mut span = one_nav_span("inline", "chromium", 105.0).remove(0);
        let attributes = span.raw["attributes"].as_array_mut().unwrap();
        attributes.retain(|attr| {
            attr.get("key").and_then(Value::as_str) != Some("e2e.navigation_top_json")
        });
        let marks = attributes
            .iter_mut()
            .find(|attr| attr.get("key").and_then(Value::as_str) == Some("e2e.boot_marks_json"))
            .unwrap();
        marks["value"]["stringValue"] = Value::String("{not json".to_owned());
        let error = boot_phase_rows(&[span]).unwrap_err();
        let detail = format!("{error:#}");
        assert!(detail.contains("e2e.boot_marks_json"), "{detail}");
        assert!(detail.contains("inline"), "{detail}");
    }

    #[test]
    fn direct_diagnostics_count_only_valid_current_completions() {
        let mut streaming = nav(1, "warm", 105.0, Some(155.0));
        streaming["wasmApiMs"] = json!(10.0);
        streaming["wasmInitMs"] = json!(20.0);
        streaming["wasmInitPath"] = json!("streaming");
        streaming["wasmExperimentArm"] = json!("shape");
        streaming["wasmModuleShape"] = shape(2, 3);

        let mut malformed = nav(2, "warm", 105.0, Some(155.0));
        malformed["wasmApiMs"] = json!(30.0);
        malformed["wasmInitMs"] = json!(20.0);
        malformed["wasmInitPath"] = json!("buffered");
        malformed["wasmExperimentArm"] = json!("shape");

        let mut legacy = dark_nav(3, "warm");
        legacy["wasmExperimentArm"] = json!("shape");
        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "chromium",
            vec![streaming, malformed, legacy],
            vec![
                (1, boot_marks(105.0)),
                (2, boot_marks(105.0)),
                (3, Vec::new()),
            ],
        ))
        .unwrap();
        let row = &rows[0];
        assert_eq!(
            (
                row.navigations,
                row.current,
                row.direct_complete,
                row.direct_missing
            ),
            (3, 2, 1, 1)
        );
        assert_eq!((row.streaming, row.buffered, row.legacy), (1, 0, 1));
        assert_eq!(
            (row.wasm_api_ms, row.wasm_init_ms),
            (Some(10.0), Some(20.0))
        );
        assert_eq!(row.experiment_arm, "shape");
        assert_eq!(
            (row.shape_complete, row.shape_missing, row.unique_shapes),
            (1, 0, 1)
        );
    }

    #[test]
    fn experiment_arm_groups_and_shape_integrity_are_reported_per_arm() {
        let mut baseline = nav(1, "warm", 105.0, Some(155.0));
        baseline["wasmApiMs"] = json!(10.0);
        baseline["wasmInitMs"] = json!(20.0);
        baseline["wasmInitPath"] = json!("streaming");
        baseline["wasmExperimentArm"] = json!("baseline");
        baseline["wasmModuleShape"] = shape(2, 3);

        let mut changed = nav(2, "warm", 110.0, Some(160.0));
        changed["wasmApiMs"] = json!(15.0);
        changed["wasmInitMs"] = json!(25.0);
        changed["wasmInitPath"] = json!("streaming");
        changed["wasmExperimentArm"] = json!("shape");
        changed["wasmModuleShape"] = shape(2, 5);

        let rows = boot_phase_rows(&test_span(
            "sqlite",
            "firefox",
            vec![changed, baseline],
            vec![(1, boot_marks(105.0)), (2, boot_marks(110.0))],
        ))
        .unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.experiment_arm.as_str())
                .collect::<Vec<_>>(),
            ["baseline", "shape"]
        );
        assert_eq!(
            rows.iter()
                .map(|row| (row.shape_complete, row.shape_missing, row.unique_shapes))
                .collect::<Vec<_>>(),
            [(1, 0, 1), (1, 0, 1)]
        );
        let report = render(&rows);
        assert!(report.contains("arm:baseline"), "{report}");
        assert!(report.contains("arm:shape"), "{report}");
        assert!(report.contains("shape complete: 1"), "{report}");
    }
}

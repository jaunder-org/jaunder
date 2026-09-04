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
    /// Navigation phase totals (`e2e.test`/`e2e.page` navigation JSON), `max_ms` desc. Section 4a.
    pub navigation_phase_hotspots: Vec<HotspotRow>,
    /// Slow navigation targets by URL path across `e2e.test`/`e2e.page`, `max_ms` desc. Section 4b.
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
    /// All schema versions, retained to make the trace input population explicit.
    pub navigations: u64,
    /// Current direct-initializer schema navigations; coverage denominator.
    pub current: u64,
    /// Pre-cutover schema navigations, excluded from current coverage loss.
    pub legacy: u64,
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

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const SLOW_STEP_MS: u128 = 1_000;

#[derive(Clone, Copy)]
pub enum Mode {
    Fix,
    Check,
}

/// Classification Nix exposes directly for a phase. `Realized` is deliberately
/// absent: a local-store transition proves neither substitution nor a local build.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NixPhaseClassification {
    Reused,
    Substituted,
    Built,
    Unknown,
}

/// A durable, machine-readable observation of one CI phase.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct PhaseRecord {
    pub name: PhaseName,
    pub duration_ms: Option<u128>,
    pub outcome: PhaseOutcome,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nix: Option<NixPhaseEvidence>,
}

impl PhaseRecord {
    pub fn unavailable(name: PhaseName, detail: impl Into<String>) -> Self {
        Self {
            name,
            duration_ms: None,
            outcome: PhaseOutcome::Unavailable,
            detail: detail.into(),
            nix: None,
        }
    }
}

/// Stable vocabulary for phase-attribution records.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhaseName {
    NixEvaluation,
    NixSubstitution,
    NixLocalBuild,
    VmStartupReadiness,
    GateExecution,
    ResultLift,
    PostGateChecks,
}

impl PhaseName {
    pub const ALL: [Self; 7] = [
        Self::NixEvaluation,
        Self::NixSubstitution,
        Self::NixLocalBuild,
        Self::VmStartupReadiness,
        Self::GateExecution,
        Self::ResultLift,
        Self::PostGateChecks,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::NixEvaluation => "nix-evaluation",
            Self::NixSubstitution => "nix-substitution",
            Self::NixLocalBuild => "nix-local-build",
            Self::VmStartupReadiness => "vm-startup-readiness",
            Self::GateExecution => "gate-execution",
            Self::ResultLift => "result-lift",
            Self::PostGateChecks => "post-gate-checks",
        }
    }
}

/// Outcome of an observable phase; unavailable is evidence of a missing boundary,
/// not a successful execution.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PhaseOutcome {
    Success,
    Failed,
    Unavailable,
}

impl PhaseOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Nix-specific phase classification and the evidence that supports it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NixPhaseEvidence {
    pub classification: NixPhaseClassification,
    pub detail: String,
}

/// The outcome of observing the selected Nix outputs across a successful build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NixRealization {
    Reused,
    Realized,
    Unknown,
}

impl NixRealization {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Reused => "reused",
            Self::Realized => "realized",
            Self::Unknown => "unknown",
        }
    }
}

/// Machine-readable identity and realization evidence for one Nix-backed step.
#[derive(Debug, Serialize)]
pub struct NixReport {
    pub installable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation: Option<String>,
    pub realization: NixRealization,
}

impl NixReport {
    /// Nix's dry-run/path-info observation proves only store reuse. It exposes
    /// no timing boundary and cannot distinguish a substitution from a build.
    pub fn phase_records(&self) -> [PhaseRecord; 3] {
        let classification = match self.realization {
            NixRealization::Reused => NixPhaseClassification::Reused,
            NixRealization::Realized | NixRealization::Unknown => NixPhaseClassification::Unknown,
        };
        let evidence = NixPhaseEvidence {
            classification,
            detail: "host dry-run/path-info observation; Nix did not expose substitution or local-build evidence".into(),
        };
        [
            PhaseRecord::unavailable(
                PhaseName::NixEvaluation,
                "nix build does not expose a separate evaluation duration",
            ),
            PhaseRecord {
                name: PhaseName::NixSubstitution,
                duration_ms: None,
                outcome: PhaseOutcome::Unavailable,
                detail: "Nix observation cannot prove whether missing outputs were substituted"
                    .into(),
                nix: Some(evidence.clone()),
            },
            PhaseRecord {
                name: PhaseName::NixLocalBuild,
                duration_ms: None,
                outcome: PhaseOutcome::Unavailable,
                detail: "Nix observation cannot prove whether missing outputs were built locally"
                    .into(),
                nix: Some(evidence),
            },
        ]
    }
}
#[derive(Debug, Serialize)]
pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub skipped: bool,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nix: Option<NixReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<PhaseRecord>,
}

impl StepResult {
    pub fn ok(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: true,
            skipped: false,
            duration_ms: 0,
            detail: None,
            nix: None,
            phases: Vec::new(),
        }
    }
    pub fn fail(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: false,
            skipped: false,
            duration_ms: 0,
            detail: None,
            nix: None,
            phases: Vec::new(),
        }
    }
    pub fn skip(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: true,
            skipped: true,
            duration_ms: 0,
            detail: None,
            nix: None,
            phases: Vec::new(),
        }
    }

    pub(crate) const fn is_blocking_failure(&self) -> bool {
        !self.ok && !self.skipped
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Attach the host-side Nix evidence for this step to the result envelope.
    pub fn nix(mut self, nix: NixReport) -> Self {
        self.nix = Some(nix);
        self
    }

    /// Attach observations produced while executing this step.
    pub fn phases(mut self, phases: impl IntoIterator<Item = PhaseRecord>) -> Self {
        self.phases.extend(phases);
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration_ms = duration.as_millis();
        self
    }

    fn human_duration(&self) -> String {
        if !self.ok || self.duration_ms >= SLOW_STEP_MS {
            format!(" ({} ms)", self.duration_ms)
        } else {
            String::new()
        }
    }

    fn human_line(&self) -> String {
        let mark = if self.skipped {
            "skip"
        } else if self.ok {
            " ok "
        } else {
            "FAIL"
        };
        let duration = self.human_duration();
        let detail = self
            .detail
            .as_deref()
            .map(|detail| format!(" — {detail}"))
            .unwrap_or_default();
        let nix = self
            .nix
            .as_ref()
            .map(|nix| match &nix.derivation {
                Some(derivation) => {
                    format!(" [nix: {} {derivation}]", nix.realization.as_str())
                }
                None => format!(" [nix: {}]", nix.realization.as_str()),
            })
            .unwrap_or_default();

        format!("[{mark}] {}{duration}{detail}{nix}", self.name)
    }
}

#[derive(Serialize)]
pub struct CommandResult {
    pub command: String,
    pub ok: bool,
    pub duration_ms: u128,
    pub finished_at_unix: u64,
    pub steps: Vec<StepResult>,
    /// The complete stable phase vocabulary. Entries begin unavailable and are
    /// replaced only by evidence collected during this command.
    pub phases: Vec<PhaseRecord>,
    /// A command-specific process status, used when xtask supervises or forwards
    /// a child whose conventional exit code must survive the result envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_override: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<crate::coverage::CoverageReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<crate::audit_wasm::AuditReport>,
    /// Per-section and per-crate attribution from `audit-wasm --breakdown`
    /// (#836). Separate from `audit` because the two describe *different
    /// artifacts* — the shipped bundle versus the unstripped pre-wasm-bindgen
    /// wasm — and merging them would invite comparing their totals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<crate::audit_wasm::BreakdownReport>,
    /// Playwright flaky tests (retried-then-passed) surfaced by `steps::flaky`
    /// from an `e2e` combo report. Empty for every other command; skipped in the
    /// sidecar when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flaky: Vec<crate::steps::flaky::FlakySpec>,
    /// Pre-rendered `traces analyze` report text. Human-facing only — `traces
    /// analyze` rejects `--json`, so this is never serialized (skipped when None,
    /// and never Some on a `--json` run).
    #[serde(skip)]
    pub traces: Option<String>,
    /// The `pr watch` / `pr land` verdict (#729). Carries the outcome an agent
    /// branches on because command-specific success cannot encode every verdict.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<crate::pr::PrReport>,
    /// `issue candidates` / `issue create` payloads (#1090/#1091).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<crate::issue::IssueReport>,
    /// The manual repository-census payload. It remains informational unless a
    /// collector itself failed, in which case its completed cells are retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub census: Option<crate::census::CensusReport>,
    /// The versioned reconciliation payload emitted by `wasm-coverage probe`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_coverage: Option<crate::wasm_coverage::Aggregate>,
}

fn render_phase_table(phases: &[PhaseRecord]) -> String {
    use std::fmt::Write as _;

    let mut out = String::from(
        "\n## CI phase attribution\n\n| Phase | Outcome | Duration | Evidence |\n| --- | --- | ---: | --- |\n",
    );
    for phase in phases {
        let duration = phase
            .duration_ms
            .map(|milliseconds| format!("{milliseconds} ms"))
            .unwrap_or_else(|| "unavailable".to_owned());
        let evidence = phase.detail.replace('|', "\\|").replace('\n', " ");
        writeln!(
            out,
            "| {} | {} | {duration} | {evidence} |",
            phase.name.as_str(),
            phase.outcome.as_str()
        )
        .unwrap();
    }
    out
}
fn render_pr_summary(pr: &crate::pr::PrReport) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    match pr.head_sha.as_str() {
        "" => writeln!(out, "PR #{} — {}", pr.pr, pr.outcome).unwrap(),
        sha => writeln!(out, "PR #{} @ {sha} — {}", pr.pr, pr.outcome).unwrap(),
    }
    if let Some(phase) = &pr.phase {
        writeln!(out, "  phase: {phase}").unwrap();
    }
    if let Some(detail) = &pr.detail {
        writeln!(out, "  {detail}").unwrap();
    }
    if let Some(pointer) = &pr.pointer {
        let label = if pr.outcome.is_merged() {
            "merge commit"
        } else {
            "see"
        };
        writeln!(out, "  {label}: {pointer}").unwrap();
    }
    out
}

impl CommandResult {
    pub fn new(command: &str) -> Self {
        Self {
            command: command.into(),
            ok: true,
            duration_ms: 0,
            finished_at_unix: 0,
            steps: Vec::new(),
            exit_override: None,
            coverage: None,
            audit: None,
            breakdown: None,
            flaky: Vec::new(),
            traces: None,
            pr: None,
            issue: None,
            census: None,
            wasm_coverage: None,
            phases: PhaseName::ALL
                .map(|name| {
                    PhaseRecord::unavailable(
                        name,
                        "this command did not observe a boundary for the phase",
                    )
                })
                .to_vec(),
        }
    }

    pub fn push(&mut self, step: StepResult) {
        self.record_phases(step.phases.iter().cloned());
        self.steps.push(step);
        self.ok = self.steps.iter().all(|s| s.ok || s.skipped);
    }

    /// Merge evidence gathered independently of the enclosing step, such as an
    /// E2E VM sidecar copied after its build has completed.
    pub fn record_phases(&mut self, phases: impl IntoIterator<Item = PhaseRecord>) {
        for phase in phases {
            let existing = self
                .phases
                .iter_mut()
                .find(|existing| existing.name == phase.name)
                .expect("phase vocabulary is initialized in CommandResult::new");
            if existing.outcome == PhaseOutcome::Unavailable {
                *existing = phase;
            } else if phase.outcome != PhaseOutcome::Unavailable {
                existing.duration_ms = match (existing.duration_ms, phase.duration_ms) {
                    (Some(left), Some(right)) => Some(left + right),
                    (duration, None) | (None, duration) => duration,
                };
                if phase.outcome == PhaseOutcome::Failed {
                    existing.outcome = PhaseOutcome::Failed;
                }
                existing.detail = format!("{}; {}", existing.detail, phase.detail);
                if existing.nix.is_none() {
                    existing.nix = phase.nix;
                }
            }
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_override.unwrap_or(if self.ok { 0 } else { 1 })
    }

    pub fn report(&self, json: bool) {
        if let Err(err) = self.write_sidecar() {
            eprintln!("xtask: warning: could not write sidecar: {err}");
        }
        if let Err(err) = self.append_github_summary() {
            eprintln!("xtask: warning: could not write GitHub step summary: {err}");
        }
        if json {
            println!("{}", serde_json::to_string_pretty(self).unwrap());
        } else {
            self.print_human();
        }
    }

    fn write_sidecar(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(".xtask")?;
        let mut f = std::fs::File::create(Path::new(".xtask/last-result.json"))?;
        f.write_all(serde_json::to_string_pretty(self).unwrap().as_bytes())?;
        Ok(())
    }

    fn append_github_summary(&self) -> std::io::Result<()> {
        let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") else {
            return Ok(());
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(render_phase_table(&self.phases).as_bytes())
    }

    fn print_human(&self) {
        for step in &self.steps {
            println!("{}", step.human_line());
        }
        // Informational payload: the audit subcommand's whole point is this table,
        // not the pass/fail line, so render it inline when present.
        if let Some(audit) = &self.audit {
            print!("{}", crate::audit_wasm::render_table(audit));
        }
        // `--breakdown`'s tables, same reasoning.
        if let Some(breakdown) = &self.breakdown {
            print!("{}", crate::audit_wasm::render_breakdown(breakdown));
        }
        // Same informational-payload treatment for `traces analyze`: the report
        // tables are the point, not the pass/fail line.
        if let Some(traces) = &self.traces {
            print!("{traces}");
        }
        // The event log already streamed to stderr; this is the stable summary seam.
        if let Some(pr) = &self.pr {
            print!("{}", render_pr_summary(pr));
        }
        if let Some(issue) = &self.issue {
            print!("{}", crate::issue::render_human(issue));
        }
        if let Some(census) = &self.census {
            print!("{}", crate::census::render_human(census));
        }
        let verdict = if self.ok { "PASSED" } else { "FAILED" };
        println!(
            "xtask {} {verdict} in {} ms",
            self.command, self.duration_ms
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_ok_reflects_steps_and_serializes_flat() {
        let mut r = CommandResult::new("validate");
        r.push(StepResult::ok("clippy").detail("0 warnings"));
        r.push(StepResult::fail("nix-coverage"));
        assert!(!r.ok);
        assert_eq!(r.exit_code(), 1);

        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["command"], "validate");
        assert_eq!(v["ok"], false);
        assert_eq!(v["steps"][0]["name"], "clippy");
        assert_eq!(v["steps"][0]["duration_ms"], 0);
        assert_eq!(v["steps"][0]["detail"], "0 warnings");
        assert!(v["steps"][0].get("nix").is_none());
        assert_eq!(v["steps"][1]["ok"], false);
        assert_eq!(v["steps"][1]["duration_ms"], 0);
    }

    #[test]
    fn nix_report_serializes_closed_realization_and_optional_derivation() {
        for (realization, spelling) in [
            (NixRealization::Reused, "reused"),
            (NixRealization::Realized, "realized"),
            (NixRealization::Unknown, "unknown"),
        ] {
            let step = StepResult::ok("nix-check").nix(NixReport {
                installable: ".#checks.x86_64-linux.xtask".into(),
                derivation: Some("/nix/store/abc-xtask.drv".into()),
                realization,
            });
            let value = serde_json::to_value(step).unwrap();

            assert_eq!(value["nix"]["installable"], ".#checks.x86_64-linux.xtask");
            assert_eq!(value["nix"]["derivation"], "/nix/store/abc-xtask.drv");
            assert_eq!(value["nix"]["realization"], spelling);
        }

        let step = StepResult::ok("nix-check").nix(NixReport {
            installable: ".#site".into(),
            derivation: None,
            realization: NixRealization::Unknown,
        });
        let value = serde_json::to_value(step).unwrap();

        assert_eq!(value["nix"]["installable"], ".#site");
        assert!(value["nix"].get("derivation").is_none());
    }

    #[test]
    fn audit_report_serializes_in_envelope() {
        let mut r = CommandResult::new("audit-wasm");
        r.push(StepResult::ok("audit-wasm").detail("2 artifact(s)"));
        r.audit = Some(crate::audit_wasm::AuditReport {
            site_path: "/nix/store/x-jaunder-csr-wasm-bundle".into(),
            artifacts: vec![crate::audit_wasm::ArtifactMetrics {
                path: "/nix/store/x-jaunder-csr-wasm-bundle/pkg/wasm-digest.wasm".into(),
                raw_bytes: 2 * 1024 * 1024,
                gzip_bytes: 700 * 1024,
                brotli_bytes: 600 * 1024,
            }],
        });
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v["audit"]["site_path"],
            "/nix/store/x-jaunder-csr-wasm-bundle"
        );
        assert_eq!(v["audit"]["artifacts"][0]["raw_bytes"], 2 * 1024 * 1024);
    }

    #[test]
    fn flaky_specs_serialize_in_envelope() {
        let mut r = CommandResult::new("e2e-sqlite-firefox");
        r.push(StepResult::ok("flaky-scan").detail("1 flaky test(s)"));
        r.flaky = vec![crate::steps::flaky::FlakySpec {
            file: "tests/visibility.spec.ts".into(),
            line: 150,
            title: "Subscriber sees the post".into(),
        }];
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["flaky"][0]["file"], "tests/visibility.spec.ts");
        assert_eq!(v["flaky"][0]["line"], 150);
        assert_eq!(v["flaky"][0]["title"], "Subscriber sees the post");
    }

    #[test]
    fn empty_flaky_is_omitted_from_json() {
        let r = CommandResult::new("check");
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert!(v.get("flaky").is_none(), "empty flaky is skipped, not `[]`");
    }

    #[test]
    fn failed_census_cell_is_retained_in_the_failing_result_envelope() {
        use crate::census::{CellReport, CellState, Language, SignalFamily};

        let mut cell = CellReport::unavailable(
            SignalFamily::DependencyStructure,
            Language::Rust,
            "fixture analyzer",
        );
        cell.state = CellState::Failed {
            error: "malformed output".into(),
        };
        let census = crate::census::CensusReport::from_cells(vec![cell]);
        let mut result = CommandResult::new("census");
        result.census = Some(census);
        result.push(StepResult::fail("census"));
        assert_eq!(result.exit_code(), 1);
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(
            value["census"]["sections"][0]["cells"][0]["state"],
            "failed"
        );
    }

    #[test]
    fn skipped_step_does_not_fail_result() {
        let mut r = CommandResult::new("check");
        r.push(StepResult::skip("clippy"));
        assert!(r.ok);
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn step_duration_serializes_and_helper_assigns_milliseconds() {
        let step = StepResult::ok("clippy").with_duration(Duration::from_millis(1234));
        assert_eq!(step.duration_ms, 1234);

        let v: serde_json::Value = serde_json::to_value(step).unwrap();
        assert_eq!(v["duration_ms"], 1234);
    }

    #[test]
    fn human_step_duration_renders_failed_or_slow_steps_only() {
        assert_eq!(StepResult::ok("fast").human_duration(), "");
        assert_eq!(
            StepResult::ok("slow")
                .with_duration(Duration::from_millis(SLOW_STEP_MS as u64))
                .human_duration(),
            format!(" ({} ms)", SLOW_STEP_MS)
        );
        assert_eq!(StepResult::fail("failed").human_duration(), " (0 ms)");
    }

    #[test]
    fn human_step_line_appends_concise_nix_state_and_derivation() {
        for (realization, spelling) in [
            (NixRealization::Reused, "reused"),
            (NixRealization::Realized, "realized"),
            (NixRealization::Unknown, "unknown"),
        ] {
            let line = StepResult::ok("nix-check")
                .nix(NixReport {
                    installable: ".#checks.x86_64-linux.xtask".into(),
                    derivation: Some("/nix/store/abc-xtask.drv".into()),
                    realization,
                })
                .human_line();

            assert_eq!(
                line,
                format!("[ ok ] nix-check [nix: {spelling} /nix/store/abc-xtask.drv]")
            );
        }
    }

    #[test]
    fn human_step_line_preserves_legacy_non_nix_output() {
        let step = StepResult::ok("clippy")
            .with_duration(Duration::from_millis(SLOW_STEP_MS as u64))
            .detail("0 warnings");

        assert_eq!(
            step.human_line(),
            format!("[ ok ] clippy ({} ms) — 0 warnings", SLOW_STEP_MS)
        );
    }
    #[test]
    fn pr_summary_renders_outcome_head_detail_and_pointer_labels() {
        let report = |outcome, detail: Option<&str>, pointer: Option<&str>| crate::pr::PrReport {
            outcome,
            pr: 1044,
            head_sha: "abc123".into(),
            phase: None,
            detail: detail.map(str::to_string),
            pointer: pointer.map(str::to_string),
            events: Vec::new(),
        };

        let ready = render_pr_summary(&report(
            crate::pr::Outcome::ReadyToLand,
            Some("obtain approval, then run `pr land`"),
            None,
        ));
        assert!(ready.contains("PR #1044 @ abc123 — ready-to-land"));
        assert!(ready.contains("obtain approval"));

        let merged = render_pr_summary(&report(crate::pr::Outcome::Merged, None, Some("deadbeef")));
        assert!(merged.contains("merged"));
        assert!(merged.contains("merge commit: deadbeef"));

        for outcome in [
            crate::pr::Outcome::Dequeued,
            crate::pr::Outcome::WatcherError,
        ] {
            let rendered = render_pr_summary(&report(outcome, Some("action required"), None));
            assert!(rendered.contains(outcome.as_str()));
            assert!(rendered.contains("abc123"));
            assert!(rendered.contains("action required"));
        }
    }
    #[test]
    fn phase_vocabulary_is_complete_and_unavailable_without_observation() {
        let result = CommandResult::new("validate");
        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(
            value["phases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|phase| phase["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "nix-evaluation",
                "nix-substitution",
                "nix-local-build",
                "vm-startup-readiness",
                "gate-execution",
                "result-lift",
                "post-gate-checks",
            ]
        );
        assert!(
            value["phases"]
                .as_array()
                .unwrap()
                .iter()
                .all(|phase| phase["outcome"] == "unavailable" && phase["duration_ms"].is_null())
        );
    }

    #[test]
    fn phase_summary_renders_the_same_record_evidence() {
        let mut result = CommandResult::new("e2e-sqlite-chromium");
        result.record_phases([PhaseRecord {
            name: PhaseName::GateExecution,
            duration_ms: Some(42),
            outcome: PhaseOutcome::Failed,
            detail: "Playwright exited 1".into(),
            nix: None,
        }]);

        let summary = render_phase_table(&result.phases);
        assert!(summary.contains("| gate-execution | failed | 42 ms | Playwright exited 1 |"));
        assert!(summary.contains("| nix-evaluation | unavailable | unavailable |"));
    }
}

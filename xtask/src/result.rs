use std::io::Write;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

const SLOW_STEP_MS: u128 = 1_000;

#[derive(Clone, Copy)]
pub enum Mode {
    Fix,
    Check,
}

#[derive(Debug, Serialize)]
pub struct StepResult {
    pub name: String,
    pub ok: bool,
    pub skipped: bool,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl StepResult {
    pub fn ok(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: true,
            skipped: false,
            duration_ms: 0,
            detail: None,
        }
    }
    pub fn fail(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: false,
            skipped: false,
            duration_ms: 0,
            detail: None,
        }
    }
    pub fn skip(name: &str) -> Self {
        Self {
            name: name.into(),
            ok: true,
            skipped: true,
            duration_ms: 0,
            detail: None,
        }
    }

    pub(crate) const fn is_blocking_failure(&self) -> bool {
        !self.ok && !self.skipped
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration_ms = duration.as_millis();
        self
    }
}

#[derive(Serialize)]
pub struct CommandResult {
    pub command: String,
    pub ok: bool,
    pub duration_ms: u128,
    pub finished_at_unix: u64,
    pub steps: Vec<StepResult>,
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
            coverage: None,
            audit: None,
            breakdown: None,
            flaky: Vec::new(),
            traces: None,
            pr: None,
            issue: None,
            census: None,
        }
    }

    pub fn push(&mut self, step: StepResult) {
        self.steps.push(step);
        self.ok = self.steps.iter().all(|s| s.ok || s.skipped);
    }

    pub fn exit_code(&self) -> i32 {
        if self.ok { 0 } else { 1 }
    }

    pub fn report(&self, json: bool) {
        if let Err(err) = self.write_sidecar() {
            eprintln!("xtask: warning: could not write sidecar: {err}");
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

    fn human_step_duration(step: &StepResult) -> String {
        if !step.ok || step.duration_ms >= SLOW_STEP_MS {
            format!(" ({} ms)", step.duration_ms)
        } else {
            String::new()
        }
    }
    fn print_human(&self) {
        for s in &self.steps {
            let mark = if s.skipped {
                "skip"
            } else if s.ok {
                " ok "
            } else {
                "FAIL"
            };
            let duration = Self::human_step_duration(s);
            let detail = s
                .detail
                .as_deref()
                .map(|d| format!(" — {d}"))
                .unwrap_or_default();
            println!("[{mark}] {}{duration}{detail}", s.name);
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
        assert_eq!(v["steps"][1]["ok"], false);
        assert_eq!(v["steps"][1]["duration_ms"], 0);
    }

    #[test]
    fn audit_report_serializes_in_envelope() {
        let mut r = CommandResult::new("audit-wasm");
        r.push(StepResult::ok("audit-wasm").detail("2 artifact(s)"));
        r.audit = Some(crate::audit_wasm::AuditReport {
            site_path: "/nix/store/x-jaunder-site".into(),
            artifacts: vec![crate::audit_wasm::ArtifactMetrics {
                path: "/nix/store/x-jaunder-site/pkg/jaunder.wasm".into(),
                raw_bytes: 2 * 1024 * 1024,
                gzip_bytes: 700 * 1024,
                brotli_bytes: 600 * 1024,
            }],
        });
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["audit"]["site_path"], "/nix/store/x-jaunder-site");
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
        assert_eq!(
            CommandResult::human_step_duration(&StepResult::ok("fast")),
            ""
        );
        assert_eq!(
            CommandResult::human_step_duration(
                &StepResult::ok("slow").with_duration(Duration::from_millis(SLOW_STEP_MS as u64))
            ),
            format!(" ({} ms)", SLOW_STEP_MS)
        );
        assert_eq!(
            CommandResult::human_step_duration(&StepResult::fail("failed")),
            " (0 ms)"
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
}

//! Coverage post-processing engine: parse the instrumented text report and the
//! CRAP report the Nix `coverage` check emits, then apply the **stateless** gate.
//!
//! The gate is history-free: an executable line FAILS iff it is uncovered AND not
//! structurally exempt (inside a `#[component]` body, see [`exempt`]) AND not
//! marked `cov:ignore` (stripped in [`report`]). A *covered* line inside an exempt
//! span trips the A1 guard (the "components are never rendered natively" invariant
//! is violated). CRAP is gated against a fixed threshold (see [`crap`]), minus
//! in-source `crap:allow` overrides. There is no baseline, anchor, or manifest.

use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::result::StepResult;

pub mod crap;
pub mod exempt;
pub mod gate;
pub mod report;

#[derive(Clone, Debug, PartialEq)]
pub struct LineCov {
    pub line: u32,
    pub covered: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileCoverage {
    pub path: String,
    pub lines: Vec<LineCov>,
}

/// The `.coverage` block of the host result envelope (`.xtask/last-result.json`):
/// the stateless gate's counts. This is NOT the Nix `status.json` (produced by
/// `devtool coverage emit` and read by CI/flake.nix) — it is the host's own
/// summary of the post-processing verdict.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CoverageReport {
    /// Uncovered, unexempt, un-ignored executable lines (each FAILS the gate).
    pub failures: usize,
    /// Covered lines inside a `#[component]` span (the A1-guard tripwire).
    pub guard_violations: usize,
    /// Functions whose CRAP exceeds the threshold with no `crap:allow` override.
    pub crap_fails: usize,
}

/// Post-process the Nix `coverage` check's `$out`: parse its text + CRAP reports
/// and apply the stateless gate.
///
/// Reads `<out_dir>/coverage-report.txt` and `<out_dir>/crap-report.json`; if
/// either is missing, returns a failed `StepResult` and `None`.
pub fn run(out_dir: &str) -> (StepResult, Option<CoverageReport>) {
    match run_inner(out_dir) {
        Ok(pair) => pair,
        Err(e) => (StepResult::fail("coverage").detail(e.to_string()), None),
    }
}

fn run_inner(out_dir: &str) -> Result<(StepResult, Option<CoverageReport>)> {
    let report_path = format!("{out_dir}/coverage-report.txt");
    let crap_path = format!("{out_dir}/crap-report.json");

    let report = match std::fs::read_to_string(&report_path) {
        Ok(s) => s,
        Err(_) => {
            return Ok((
                StepResult::fail("coverage")
                    .detail(format!("missing coverage report at {report_path}")),
                None,
            ));
        }
    };
    let crap_report_str = match std::fs::read_to_string(&crap_path) {
        Ok(s) => s,
        Err(_) => {
            return Ok((
                StepResult::fail("coverage").detail(format!("missing CRAP report at {crap_path}")),
                None,
            ));
        }
    };

    let repo_root = git_repo_root()?;
    let current = report::parse_text_report(&report, &repo_root)?;

    // The stateless coverage gate (#231): an executable line fails iff uncovered,
    // not structurally exempt (`#[component]`), and not `cov:ignore`'d (the latter
    // is already stripped by `parse_text_report`). A covered line inside an exempt
    // span trips the A1 guard. `exempt_of` reads each repo-relative source file
    // from `repo_root` and returns its exempt lines, or an EMPTY set on any
    // read/parse error (fail-closed: unknown → measured).
    let exempt_of = |path: &str| -> std::collections::BTreeSet<u32> {
        let full = std::path::Path::new(&repo_root).join(path);
        match std::fs::read_to_string(&full) {
            Ok(src) => exempt::exempt_lines(&src).unwrap_or_default(),
            Err(_) => std::collections::BTreeSet::new(),
        }
    };
    let verdict = gate::evaluate(&current, exempt_of);
    write_failures_dump(&verdict);

    // The CRAP threshold gate (#231/#232): fail any function whose CRAP exceeds
    // the threshold, minus an in-source `crap:allow` override. Each over-threshold
    // function's source is read (relative to `repo_root`) to honor the override.
    let allow = crap::AllowSet::new(|file: &str| {
        std::fs::read_to_string(std::path::Path::new(&repo_root).join(file)).ok()
    });
    let entries = crap::parse_entries(&crap_report_str).context("parsing CRAP report")?;
    let crap_fails = crap::evaluate_crap(&entries, &allow);

    let gate_fails = !verdict.failures.is_empty()
        || !verdict.guard_violations.is_empty()
        || !crap_fails.is_empty();

    let report = CoverageReport {
        failures: verdict.failures.len(),
        guard_violations: verdict.guard_violations.len(),
        crap_fails: crap_fails.len(),
    };

    let step = if gate_fails {
        StepResult::fail("coverage").detail(failure_report(&verdict, &crap_fails))
    } else {
        let checked: usize = current.iter().map(|f| f.lines.len()).sum();
        StepResult::ok("coverage").detail(format!(
            "clean — {checked} executable line(s), 0 failures, 0 guard violations, 0 CRAP over threshold",
        ))
    };

    Ok((step, Some(report)))
}

/// Dump the gate's full failures list (one `path:line` per line, sorted by path
/// then line) to `.xtask/coverage-failures.txt` — a machine-checkable worklist of
/// every uncovered-unexempt line. Best-effort: a write error must not perturb the
/// run.
fn write_failures_dump(verdict: &gate::Verdict) {
    let mut lines: Vec<(&str, u32)> = verdict
        .failures
        .iter()
        .map(|f| (f.file.as_str(), f.line))
        .collect();
    lines.sort();
    let mut body = String::new();
    for (file, line) in &lines {
        use std::fmt::Write as _;
        let _ = writeln!(body, "{file}:{line}");
    }
    let _ = std::fs::create_dir_all(".xtask");
    let _ = std::fs::write(".xtask/coverage-failures.txt", body);
}

/// Render a coverage-gate failure as a concise, actionable report: each uncovered
/// line and A1-guard violation as `file:line: text`, each CRAP fail as
/// `file::fn crap=<v>`, plus what to do — so the invoker never has to read the raw
/// report by hand (#87/#88). Capped so a large failure stays one screen; the count
/// and "… N more" make the truncation explicit.
fn failure_report(verdict: &gate::Verdict, crap_fails: &[crap::CrapFail]) -> String {
    use std::fmt::Write as _;
    const MAX: usize = 25;
    let mut s = format!(
        "{} uncovered line(s), {} guard violation(s), {} CRAP over threshold",
        verdict.failures.len(),
        verdict.guard_violations.len(),
        crap_fails.len(),
    );
    if !verdict.failures.is_empty() {
        s.push_str("\n  uncovered (not #[component]-exempt, not cov:ignore'd):");
        for f in verdict.failures.iter().take(MAX) {
            let _ = write!(s, "\n    {}:{}: {}", f.file, f.line, f.text.trim());
        }
        if verdict.failures.len() > MAX {
            let _ = write!(s, "\n    … and {} more", verdict.failures.len() - MAX);
        }
    }
    if !verdict.guard_violations.is_empty() {
        s.push_str("\n  A1-guard — covered line inside a #[component] span:");
        for g in verdict.guard_violations.iter().take(MAX) {
            let _ = write!(s, "\n    {}:{}: {}", g.file, g.line, g.text.trim());
        }
        if verdict.guard_violations.len() > MAX {
            let _ = write!(
                s,
                "\n    … and {} more",
                verdict.guard_violations.len() - MAX
            );
        }
    }
    if !crap_fails.is_empty() {
        s.push_str("\n  CRAP over threshold:");
        for c in crap_fails.iter().take(MAX) {
            let _ = write!(s, "\n    {}::{} crap={:.2}", c.file, c.function, c.crap);
        }
        if crap_fails.len() > MAX {
            let _ = write!(s, "\n    … and {} more", crap_fails.len() - MAX);
        }
    }
    if !verdict.failures.is_empty() {
        s.push_str(
            "\n  → add a test covering these lines, or mark accepted-uncovered with a trailing\
             \n    `// cov:ignore` (single line) or a `// cov:ignore-start` / `// cov:ignore-stop` block.",
        );
    }
    if !verdict.guard_violations.is_empty() {
        s.push_str(
            "\n  → a #[component] body is being exercised natively, so the blanket component\
             \n    exemption discards REAL coverage — revisit the exemption (spec §A1-guard).",
        );
    }
    if !crap_fails.is_empty() {
        s.push_str(
            "\n  → reduce the function's complexity or improve its coverage; if this is approved\
             \n    drift, add `// crap:allow: <reason>` within the function's span.",
        );
    }
    s
}

fn git_repo_root() -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("running git rev-parse --show-toplevel")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fail(file: &str, line: u32, text: &str) -> gate::Fail {
        gate::Fail {
            file: file.into(),
            line,
            text: text.into(),
        }
    }

    #[test]
    fn failure_report_lists_uncovered_guard_and_crap() {
        let verdict = gate::Verdict {
            failures: vec![fail("a.rs", 10, "    let x = bar()?;")],
            guard_violations: vec![fail("c.rs", 3, "view! { <div/> }")],
        };
        let crap = vec![crap::CrapFail {
            file: "b.rs".into(),
            function: "big".into(),
            line: 5,
            crap: 42.0,
        }];
        let r = failure_report(&verdict, &crap);
        assert!(r.contains("1 uncovered line(s), 1 guard violation(s), 1 CRAP over threshold"));
        assert!(r.contains("a.rs:10: let x = bar()?;"), "{r}"); // text trimmed
        assert!(r.contains("c.rs:3: view! { <div/> }"), "{r}");
        assert!(r.contains("b.rs::big crap=42.00"), "{r}");
        assert!(r.contains("cov:ignore"), "uncovered guidance: {r}");
        assert!(r.contains("crap:allow"), "crap guidance: {r}");
        assert!(r.contains("revisit the exemption"), "guard guidance: {r}");
    }

    #[test]
    fn failure_report_guidance_is_category_conditional() {
        // A CRAP-only failure must not show the coverage-lowering / guard guidance.
        let crap = vec![crap::CrapFail {
            file: "b.rs".into(),
            function: "f".into(),
            line: 1,
            crap: 99.0,
        }];
        let r = failure_report(&gate::Verdict::default(), &crap);
        assert!(!r.contains("uncovered ("), "{r}");
        assert!(!r.contains("A1-guard"), "{r}");
        assert!(r.contains("crap:allow"), "{r}");
    }

    #[test]
    fn failure_report_caps_long_lists() {
        let failures: Vec<_> = (0..30).map(|i| fail("a.rs", i, "x")).collect();
        let verdict = gate::Verdict {
            failures,
            guard_violations: vec![],
        };
        let r = failure_report(&verdict, &[]);
        assert!(r.contains("30 uncovered line(s)"));
        assert!(r.contains("… and 5 more"), "{r}"); // 30 - cap 25
    }
}

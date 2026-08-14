use std::collections::BTreeMap;
use std::io::Write as IoWrite;
use std::path::Path;

use anyhow::{Context, Result};

use super::{CoverageReport, crap, exempt, gate, report};
use crate::git;
use crate::result::StepResult;

/// Post-process the Nix `coverage` check's `$out`: parse its text + CRAP reports
/// and apply the stateless gate.
///
/// Reads `<out_dir>/coverage-report.txt` and `<out_dir>/crap-report.json`; if
/// either is missing, returns a failed `StepResult` and `None`.
pub fn run(out_dir: &str) -> (StepResult, Option<CoverageReport>) {
    match run_inner(out_dir) {
        Ok(pair) => pair,
        Err(error) => (coverage_failure_step(error), None),
    }
}
fn coverage_failure_step(error: anyhow::Error) -> StepResult {
    StepResult::fail("coverage").detail(format!("{error:#}"))
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

    let repo_root = git::toplevel(Path::new("."))?;
    let current = report::parse_text_report(&report, &repo_root)?;

    // Every report source must be readable before the verdict is evaluated.
    // Per ADR-0050, syntax failures carry no exemption evidence and therefore
    // leave the source fully measured via an empty exemption set.
    let exemptions =
        exemption_population(&current, &repo_root, |path| std::fs::read_to_string(path))?;
    let verdict = gate::evaluate(&current, |path| {
        exemptions.get(path).cloned().unwrap_or_default()
    });
    write_failures_dump(&verdict);

    // The CRAP threshold gate (#231/#232): fail any function whose CRAP exceeds
    // the threshold, minus an in-source `crap:allow` override. Each over-threshold
    // function's source is read (relative to `repo_root`) to honor the override.
    let allow = crap::AllowSet::new(|file: &str| {
        let path = std::path::Path::new(&repo_root).join(file);
        std::fs::read_to_string(&path)
            .with_context(|| format!("reading CRAP allow-marker source {}", path.display()))
    });
    let entries = crap::parse_entries(&crap_report_str).context("parsing CRAP report")?;
    let crap_fails = crap::evaluate_crap(&entries, &allow)?;

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

/// Dump the gate's full failures list. Diagnostic persistence is best-effort,
/// but failure is visible as one fixed warning and never changes the gate result.
fn write_failures_dump(verdict: &gate::Verdict) {
    write_failures_dump_with(
        verdict,
        |path| std::fs::create_dir_all(path),
        |path, body| std::fs::write(path, body),
        &mut std::io::stderr(),
    );
}

fn write_failures_dump_with(
    verdict: &gate::Verdict,
    create_dir: impl FnOnce(&Path) -> std::io::Result<()>,
    write_file: impl FnOnce(&Path, String) -> std::io::Result<()>,
    stderr: &mut impl IoWrite,
) {
    let mut lines: Vec<(&str, u32)> = verdict
        .failures
        .iter()
        .map(|failure| (failure.file.as_str(), failure.line))
        .collect();
    lines.sort();
    let mut body = String::new();
    for (file, line) in &lines {
        use std::fmt::Write as _;
        let _ = writeln!(body, "{file}:{line}");
    }
    let outcome = create_dir(Path::new(".xtask"))
        .and_then(|()| write_file(Path::new(".xtask/coverage-failures.txt"), body));
    if outcome.is_err() {
        let _ = writeln!(
            stderr,
            "xtask: warning: xtask.coverage.failure_dump: ignored failure while writing coverage diagnostic dump"
        );
    }
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
        s.push_str("\n  uncovered (not an unreachable!(\"msg\"), not cov:ignore'd):");
        for f in verdict.failures.iter().take(MAX) {
            let _ = write!(s, "\n    {}:{}: {}", f.file, f.line, f.text.trim());
        }
        if verdict.failures.len() > MAX {
            let _ = write!(s, "\n    … and {} more", verdict.failures.len() - MAX);
        }
    }
    if !verdict.guard_violations.is_empty() {
        s.push_str("\n  A1-guard — covered line inside an unreachable! span:");
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
            "\n  → an `unreachable!` assertion was actually reached, so its premise is\
             \n    violated — revisit the exemption (spec §A1-guard).",
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

fn exemption_population(
    current: &[crate::coverage::FileCoverage],
    repo_root: &str,
    mut read_source: impl FnMut(&Path) -> std::io::Result<String>,
) -> Result<BTreeMap<String, std::collections::BTreeSet<u32>>> {
    current
        .iter()
        .map(|file| {
            let full = Path::new(repo_root).join(&file.path);
            let source = read_source(&full)
                .with_context(|| format!("reading coverage exemption source {}", full.display()))?;
            // ADR-0050: malformed syntax proves no exemption; it does not remove
            // the file from the measured population.
            let lines = exempt::exempt_lines(&source).unwrap_or_default();
            Ok((file.path.clone(), lines))
        })
        .collect::<Result<_>>()
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

    #[test]
    fn fail_closed_population_unreadable_coverage_exemption_source() {
        let current = vec![crate::coverage::FileCoverage {
            path: "secret.rs".to_owned(),
            lines: Vec::new(),
        }];
        let error = exemption_population(&current, "/repo", |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected",
            ))
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("/repo/secret.rs"));
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
        let step = coverage_failure_step(error);
        assert!(!step.ok);
        let detail = step.detail.unwrap();
        assert!(detail.contains("/repo/secret.rs"), "{detail}");
        assert!(detail.contains("injected"), "{detail}");
    }

    #[test]
    fn ancillary_warning_diagnostic_dump_failures_preserve_verdict() {
        let verdict = gate::Verdict {
            failures: vec![fail("a.rs", 1, "x")],
            guard_violations: Vec::new(),
        };
        for fail_create in [true, false] {
            let mut stderr = Vec::new();
            write_failures_dump_with(
                &verdict,
                |_| {
                    if fail_create {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "injected create",
                        ))
                    } else {
                        Ok(())
                    }
                },
                |_, _| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "injected write",
                    ))
                },
                &mut stderr,
            );
            assert_eq!(verdict.failures.len(), 1);
            let warning = String::from_utf8(stderr).unwrap();
            assert_eq!(warning.matches("xtask.coverage.failure_dump").count(), 1);
            assert_eq!(warning.lines().count(), 1);
            assert!(!warning.contains("injected"));
        }
    }
}

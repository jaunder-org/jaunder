use std::collections::BTreeMap;
use std::fs;
use std::io::Write as IoWrite;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{CoverageReport, crap, exempt, gate, report};
use host_tool::report::StepResult;
use repository_policy::git;

/// Apply only host source policy to realized text + CRAP reports. The Just
/// workflow must pass the hard Nix producer-status gate before invoking this leaf.
pub fn run(out_dir: &Path) -> (StepResult, Option<CoverageReport>) {
    match run_inner(out_dir) {
        Ok(pair) => pair,
        Err(error) => (coverage_failure_step(error), None),
    }
}
fn coverage_failure_step(error: anyhow::Error) -> StepResult {
    StepResult::fail("coverage").detail(format!("{error:#}"))
}

fn run_inner(out_dir: &Path) -> Result<(StepResult, Option<CoverageReport>)> {
    let report_path = out_dir.join("coverage-report.txt");
    let crap_path = out_dir.join("crap-report.json");
    let report = fs::read_to_string(&report_path)
        .with_context(|| format!("reading coverage report at {}", report_path.display()))?;

    if report.trim().is_empty() {
        bail!("empty coverage report at {}", report_path.display());
    }

    let repo_root = git::toplevel(Path::new("."))?;
    let current = report::parse_text_report(&report, &repo_root)?;
    let checked: usize = current.iter().map(|file| file.lines.len()).sum();
    if checked == 0 {
        bail!("coverage report has zero executable lines");
    }

    // Every report source must be readable before the verdict is evaluated.
    // Per ADR-0050, syntax failures carry no exemption evidence and therefore
    // leave the source fully measured via an empty exemption set.
    let exemptions = exemption_population(&current, &repo_root, |path| fs::read_to_string(path))?;

    let crap_report_str = fs::read_to_string(&crap_path)
        .with_context(|| format!("reading CRAP report at {}", crap_path.display()))?;

    let verdict = gate::evaluate(&current, |path| {
        exemptions.get(path).cloned().unwrap_or_default()
    });
    write_failures_dump(&verdict);

    // The CRAP threshold gate (#231/#232): fail any function whose CRAP exceeds
    // the threshold, minus an in-source `crap:allow` override. Each over-threshold
    // function's source is read (relative to `repo_root`) to honor the override.
    let allow = crap::AllowSet::new(|file: &str| {
        let path = std::path::Path::new(&repo_root).join(file);
        fs::read_to_string(&path)
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
        |path| fs::create_dir_all(path),
        |path, body| fs::write(path, body),
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
             \n    `// cov:ignore: <specific reason>` (single line) or\
             \n    `// cov:ignore-start: <specific reason>` … `// cov:ignore-stop` block.",
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
    current: &[crate::host::policy::FileCoverage],
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

    fn write_reports(out: &std::path::Path, text_report: &str, crap_report: &str) {
        std::fs::write(out.join("coverage-report.txt"), text_report).unwrap();
        std::fs::write(out.join("crap-report.json"), crap_report).unwrap();
    }

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
        assert!(
            r.contains("cov:ignore: <specific reason>")
                && r.contains("cov:ignore-start: <specific reason>")
                && r.contains("cov:ignore-stop"),
            "uncovered guidance: {r}"
        );
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
        let current = vec![crate::host::policy::FileCoverage {
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

    #[test]
    fn run_rejects_missing_or_empty_text_evidence_before_crap_evaluation() {
        let missing = tempfile::tempdir().unwrap();
        let (step, _) = run(missing.path());
        assert!(!step.ok);
        assert!(
            step.detail.unwrap().contains("reading coverage report"),
            "missing text evidence must be named"
        );

        let empty = tempfile::tempdir().unwrap();
        write_reports(empty.path(), "", "not JSON");
        let (step, _) = run(empty.path());
        assert!(!step.ok);
        let detail = step.detail.unwrap();
        assert!(
            detail.contains("empty coverage report"),
            "empty text evidence must fail before CRAP parsing: {detail}"
        );
    }

    #[test]
    fn run_rejects_zero_executable_lines_before_crap_evaluation() {
        let out = tempfile::tempdir().unwrap();
        let repo_root = repository_policy::git::toplevel(std::path::Path::new(".")).unwrap();
        write_reports(
            out.path(),
            &format!("{repo_root}/tools/coverage/src/host/policy/run.rs:\n"),
            "not JSON",
        );
        let (step, _) = run(out.path());

        assert!(!step.ok);
        let detail = step.detail.unwrap();
        assert!(
            detail.contains("zero executable lines"),
            "zero-line evidence must fail before CRAP parsing: {detail}"
        );
    }

    #[test]
    fn policy_leaf_preserves_source_exceptions_a1_and_exclusive_crap_threshold() {
        for (source, count, score, passes) in [
            ("    ordinary();", "0", 0.0, false),
            (
                "    ordinary(); // cov:ignore: target-only path",
                "0",
                0.0,
                true,
            ),
            ("    ordinary(); // cov:ignore", "0", 0.0, false),
            ("    unreachable!(\"caller invariant\");", "0", 0.0, true),
            ("    unreachable!(\"caller invariant\");", "1", 0.0, false),
            (
                "    std::unreachable!(\"unsupported path\");",
                "0",
                0.0,
                false,
            ),
            ("    ordinary();", "1", 30.0, true),
            ("    ordinary();", "1", 30.01, false),
            (
                "    ordinary(); // crap:allow: bounded dispatch",
                "1",
                30.01,
                true,
            ),
        ] {
            let out = tempfile::tempdir().unwrap();
            let path = out.path().join("sample.rs");
            fs::write(&path, format!("fn sample() {{\n{source}\n}}\n")).unwrap();
            write_reports(out.path(),
                &format!("{}:\n    1|     1|fn sample() {{\n    2|{count}|{source}\n    3|     1|}}\n", path.display()),
                &serde_json::json!({"entries": [{"file": path, "function": "sample", "line": 1, "crap": score}]}).to_string(),
            );
            let (step, _) = run(out.path());
            assert_eq!(
                step.ok, passes,
                "{source} count={count} CRAP={score}: {:?}",
                step.detail
            );
        }
    }

    #[test]
    fn policy_leaf_rejects_malformed_text_and_missing_or_malformed_crap() {
        for text in ["not a line table", "/repo/a.rs:\n1|bogus|uncovered()\n"] {
            let out = tempfile::tempdir().unwrap();
            write_reports(out.path(), text, r#"{"entries":[]}"#);
            assert!(!run(out.path()).0.ok);
        }
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("sample.rs");
        fs::write(&path, "fn sample() {}\n").unwrap();
        fs::write(
            out.path().join("coverage-report.txt"),
            format!("{}:\n1|1|fn sample() {{}}\n", path.display()),
        )
        .unwrap();
        assert!(!run(out.path()).0.ok);
        fs::write(out.path().join("crap-report.json"), "not JSON").unwrap();
        assert!(!run(out.path()).0.ok);
    }

    #[test]
    fn policy_leaf_accepts_complete_reports_without_producer_status() {
        let out = tempfile::tempdir().unwrap();
        let repo_root = repository_policy::git::toplevel(std::path::Path::new(".")).unwrap();
        write_reports(
            out.path(),
            &format!(
                "{repo_root}/tools/coverage/src/host/policy/run.rs:\n    1|     1|use std::collections::BTreeMap;\n"
            ),
            r#"{"entries":[]}"#,
        );
        let (step, report) = run(out.path());
        assert!(step.ok, "{:?}", step.detail);
        assert_eq!(report, Some(CoverageReport::default()));
    }
}

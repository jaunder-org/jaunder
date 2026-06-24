use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use coverage::status::{CoverageStatus, StatusCategory};

/// Classify captured `cargo llvm-cov nextest` output into the in-sandbox
/// sentinel. Infra failures (disk/OOM) take precedence over test failures,
/// because a disk-full run ALSO produces spurious test FAILs (#28).
pub fn classify_nextest_output(output: &str) -> CoverageStatus {
    const INFRA_MARKERS: &[&str] = &[
        "No space left on device",
        "53100",
        "Cannot allocate memory",
        "out of memory",
    ];
    if let Some(marker) = INFRA_MARKERS.iter().find(|m| output.contains(**m)) {
        return CoverageStatus {
            category: StatusCategory::Infra,
            failed_tests: Vec::new(),
            infra_detail: Some((*marker).to_string()),
        };
    }
    let failed_tests: Vec<String> = output
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            // nextest summary line: "FAIL [   0.71s] <suite> <test path>"
            let rest = l.strip_prefix("FAIL [")?;
            let after_bracket = rest.split(']').nth(1)?.trim();
            after_bracket.split_whitespace().last().map(str::to_string)
        })
        .collect();
    if failed_tests.is_empty() {
        CoverageStatus {
            category: StatusCategory::TestsOk,
            failed_tests,
            infra_detail: None,
        }
    } else {
        CoverageStatus {
            category: StatusCategory::TestFailure,
            failed_tests,
            infra_detail: None,
        }
    }
}

/// Run the instrumented suite and emit reports + status + diagnostics into `out`.
/// Always produces `out/status.json` (and best-effort the rest) so the caller's
/// Nix producer derivation can always realize `$out`. Returns `Err` only if the
/// emit could not run at all (e.g. failing to spawn cargo).
pub fn run(out: &str) -> Result<()> {
    let out = Path::new(out);
    let diag = out.join("diagnostics");
    fs::create_dir_all(&diag).with_context(|| format!("creating {}", diag.display()))?;

    let abs_root = std::env::current_dir()?.to_string_lossy().to_string();

    // 1. Clear stale profraw, keep the instrumented build.
    run_logged(Command::new("cargo").args(["llvm-cov", "clean", "--profraw-only"]))?;

    // 2. Instrumented suite under an ephemeral PostgreSQL. Capture combined
    //    output for classification + the diagnostics bundle. A non-zero exit is
    //    NOT fatal here: a test failure or infra failure is reported via status.
    let nextest = run_capture(Command::new("bash").args([
        "scripts/with-ephemeral-postgres",
        "cargo",
        "llvm-cov",
        "--no-report",
        "nextest",
        "--show-progress",
        "none",
    ]))?;
    fs::write(diag.join("nextest.log"), &nextest)?;
    let status = classify_nextest_output(&nextest);
    fs::write(out.join("status.json"), status.to_json())?;

    // 3. Disk-usage snapshot for the diagnostics bundle (#28).
    let df = run_capture(Command::new("df").arg("-h"))?;
    fs::write(diag.join("disk-usage.txt"), df)?;

    // 4. Text + LCOV reports (best-effort; on infra failure they may be partial).
    let report = run_capture(Command::new("cargo").args(["llvm-cov", "report", "--text"]))?;
    let report = coverage::pathnorm::normalize_report_text(&report, &abs_root);
    fs::write(out.join("coverage-report.txt"), &report)?;

    let lcov = out.join("coverage-report.lcov");
    run_logged(Command::new("cargo").args([
        "llvm-cov",
        "report",
        "--lcov",
        "--output-path",
        lcov.to_str().unwrap(),
    ]))?;

    // 5. CRAP report, normalized to repo-relative file paths.
    let raw_crap = out.join("crap-report.raw.json");
    run_logged(Command::new("cargo").args([
        "crap",
        "--workspace",
        "--lcov",
        lcov.to_str().unwrap(),
        "--exclude",
        "**/tests/**",
        "--format",
        "json",
        "--output",
        raw_crap.to_str().unwrap(),
    ]))?;
    // Best-effort: a failed or absent CRAP report must NOT abort the emit — the
    // producer always succeeds and the gate reads status.json. Default to an
    // empty report so `crap-report.json` always exists for the install phase.
    let crap_json = fs::read_to_string(&raw_crap)
        .ok()
        .and_then(|raw| normalize_crap_paths(&raw, &abs_root).ok())
        .unwrap_or_else(|| "{\n  \"entries\": []\n}\n".to_string());
    fs::write(out.join("crap-report.json"), crap_json)?;

    Ok(())
}

/// Strip the absolute sandbox prefix from each CRAP entry's `.file` (ports the
/// `jq` rewrite in `normalize_crap_report`).
fn normalize_crap_paths(raw: &str, abs_root: &str) -> Result<String> {
    let prefix = format!("{abs_root}/");
    let mut v: serde_json::Value = serde_json::from_str(raw)?;
    if let Some(entries) = v.get_mut("entries").and_then(|e| e.as_array_mut()) {
        for e in entries {
            if let Some(f) = e.get("file").and_then(|f| f.as_str()) {
                let rel = f.strip_prefix(&prefix).unwrap_or(f).to_string();
                e["file"] = serde_json::Value::String(rel);
            }
        }
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&v)?))
}

/// Spawn, inheriting stdio, erroring if the process could not be launched.
fn run_logged(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("spawning {cmd:?}"))?;
    // A non-zero exit is tolerated (recorded elsewhere); a spawn failure is not.
    let _ = status;
    Ok(())
}

/// Spawn, capturing combined stdout+stderr as a String.
fn run_capture(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().with_context(|| format!("spawning {cmd:?}"))?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_disk_full_as_infra_even_with_fails() {
        let out = "\
FAIL [ 0.71s] jaunder::web web_posts::case_3
could not extend file \"base/25350/2609_vm\": No space left on device
";
        let s = classify_nextest_output(out);
        assert_eq!(s.category, StatusCategory::Infra);
        assert_eq!(s.infra_detail.as_deref(), Some("No space left on device"));
    }

    #[test]
    fn collects_failed_test_names() {
        let out = "\
FAIL [ 0.71s] jaunder::web web_posts::endpoint_rejects_unauthenticated::case_3
FAIL [ 0.04s] jaunder::web web_posts::get_post_carries_tags::case_2_postgres
";
        let s = classify_nextest_output(out);
        assert_eq!(s.category, StatusCategory::TestFailure);
        assert_eq!(s.failed_tests.len(), 2);
        assert!(s.failed_tests[0].ends_with("case_3"));
    }

    #[test]
    fn clean_output_is_tests_ok() {
        let out = "Summary [ 34s] 1531/1531 tests run: 1531 passed";
        assert_eq!(
            classify_nextest_output(out).category,
            StatusCategory::TestsOk
        );
    }

    #[test]
    fn normalize_crap_paths_strips_prefix() {
        let raw = r#"{"entries":[{"file":"/build/source/server/src/a.rs","crap":1.0}]}"#;
        let got = super::normalize_crap_paths(raw, "/build/source").unwrap();
        assert!(got.contains("\"server/src/a.rs\""));
        assert!(!got.contains("/build/source"));
    }
}

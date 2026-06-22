//! Coverage post-processing engine: parse the instrumented text report and the
//! CRAP report the Nix `coverage` check emits, classify each line-coverage delta
//! against the committed baseline, gate on regressions, and (in `Mode::Fix`)
//! auto-heal the shrink-only baseline + CRAP manifest.

use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::result::{Mode, StepResult};

pub mod baseline;
pub mod classify;
pub mod crap;
pub mod diffmap;
pub mod report;

use baseline::Baseline;
use crap::CrapRegression;

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

/// A file plus a set of its line numbers — the unit reported in each verdict
/// bucket. Lines are kept sorted for stable output/diffs.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FileLines {
    pub file: String,
    pub lines: Vec<u32>,
}

/// The classifier's verdict: each delta bucketed by line identity.
/// `regressions` (previously-covered line now uncovered) and `new_uncovered`
/// (brand-new uncovered line) both FAIL the gate; `structural` (a baseline gap
/// whose line was deleted) and `improvements` (a baseline gap now covered) are
/// safe deltas the gate auto-heals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageVerdict {
    pub regressions: Vec<FileLines>,
    pub new_uncovered: Vec<FileLines>,
    pub structural: Vec<FileLines>,
    pub improvements: Vec<FileLines>,
}

impl CoverageVerdict {
    /// Clean iff there is nothing that fails the gate: no regressions and no
    /// new uncovered lines. (structural/improvements are heals, not failures.)
    pub fn is_clean(&self) -> bool {
        self.regressions.is_empty() && self.new_uncovered.is_empty()
    }
}

/// The CRAP block of the coverage report envelope.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CrapReport {
    pub regressions: Vec<CrapRegression>,
}

/// The `.coverage` block of the result envelope: every classified delta bucket,
/// the CRAP regressions, and whether the baselines were auto-healed this run.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CoverageReport {
    pub regressions: Vec<FileLines>,
    pub new_uncovered: Vec<FileLines>,
    pub structural: Vec<FileLines>,
    pub improvements: Vec<FileLines>,
    pub crap: CrapReport,
    pub healed: bool,
}

const BASELINE_PATH: &str = "coverage-baseline.json";
const CRAP_MANIFEST_PATH: &str = "crap-manifest.json";

/// Decide whether to heal the accepted-uncovered baseline, returning the
/// (possibly) new baseline to persist and the `healed` flag.
///
/// Heal happens ONLY when the run is clean: `Mode::Fix`, no gate-failing
/// coverage delta (`verdict.is_clean()`), and no CRAP regressions. When clean,
/// the new baseline is simply `Baseline::from_files(&current)` — "clean" means
/// every current uncovered line is an already-accepted gap, so regenerating from
/// the current report drops improved (now-covered) and structural (deleted) gaps
/// and re-numbers survivors to working-tree lines, and never adds a new gap
/// (shrink-only holds by construction). We only persist when it differs from the
/// loaded baseline.
///
/// Returns `(None, false)` whenever anything fails or in `Mode::Check`.
fn heal_baseline(
    verdict: &CoverageVerdict,
    crap_regs: &[CrapRegression],
    current: &[FileCoverage],
    loaded: &Baseline,
    mode: Mode,
) -> (Option<Baseline>, bool) {
    let clean = verdict.is_clean() && crap_regs.is_empty();
    if !matches!(mode, Mode::Fix) || !clean {
        return (None, false);
    }
    let healed = Baseline::from_files(current);
    if healed.to_json() != loaded.to_json() {
        (Some(healed), true)
    } else {
        (None, false)
    }
}

/// Post-process the Nix `coverage` check's `$out`: parse its text + CRAP reports,
/// classify against the committed baselines, gate, and (in `Mode::Fix`) heal.
///
/// Reads `<out_dir>/coverage-report.txt` and `<out_dir>/crap-report.json`; if
/// either is missing, returns a failed `StepResult` and `None`.
pub fn run(out_dir: &str, mode: Mode) -> (StepResult, Option<CoverageReport>) {
    match run_inner(out_dir, mode) {
        Ok(pair) => pair,
        Err(e) => (StepResult::fail("coverage").detail(e.to_string()), None),
    }
}

fn run_inner(out_dir: &str, mode: Mode) -> Result<(StepResult, Option<CoverageReport>)> {
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
    let current = report::parse_text_report(&report, &repo_root);

    let diff = git_diff_unified0()?;
    let mut maps = diffmap::parse_unified_diff(&diff);

    // `git diff HEAD` omits untracked files, so a brand-new untracked file gets
    // no map → identity → its uncovered lines would be mislabeled `regression`.
    // Map every untracked file present in the current report as all-added so its
    // uncovered lines classify as `new_uncovered`. Never overwrite a map already
    // built from the diff.
    for path in git_untracked_files()? {
        if maps.contains_key(&path) {
            continue;
        }
        if let Some(f) = current.iter().find(|f| f.path == path) {
            let lines: Vec<u32> = f.lines.iter().map(|l| l.line).collect();
            maps.insert(path, diffmap::LineMap::all_added(&lines));
        }
    }

    let baseline = Baseline::load(BASELINE_PATH)?;
    let verdict = classify::classify(&current, &baseline, &maps);

    let old_crap_manifest = std::fs::read_to_string(CRAP_MANIFEST_PATH).unwrap_or_default();
    let crap_regs = if old_crap_manifest.trim().is_empty() {
        Vec::new()
    } else {
        crap::compare(&crap_report_str, &old_crap_manifest)?
    };

    let gate_fails = !verdict.is_clean() || !crap_regs.is_empty();

    // Auto-heal — only when clean (Mode::Fix), never on failure.
    let (new_baseline, mut healed) = heal_baseline(&verdict, &crap_regs, &current, &baseline, mode);
    if let Some(b) = &new_baseline {
        b.save(BASELINE_PATH)?;
    }
    // Likewise regenerate the CRAP manifest when there are no CRAP regressions
    // (Mode::Fix) and it differs from the committed manifest.
    if matches!(mode, Mode::Fix) && verdict.is_clean() && crap_regs.is_empty() {
        // Compare in canonical (minified, key-sorted) form so equality is
        // independent of on-disk formatting, but WRITE pretty-printed so the
        // committed manifest stays human-diffable (a one-line blob makes
        // coverage diffs unreadable). The baseline is likewise written pretty.
        if normalize_json(&crap_report_str)? != normalize_json_or_empty(&old_crap_manifest) {
            std::fs::write(CRAP_MANIFEST_PATH, pretty_json(&crap_report_str)?)
                .with_context(|| format!("writing {CRAP_MANIFEST_PATH}"))?;
            healed = true;
        }
    }

    let report = CoverageReport {
        regressions: verdict.regressions.clone(),
        new_uncovered: verdict.new_uncovered.clone(),
        structural: verdict.structural.clone(),
        improvements: verdict.improvements.clone(),
        crap: CrapReport {
            regressions: crap_regs.clone(),
        },
        healed,
    };

    let step = if gate_fails {
        let detail = format!(
            "{} regression(s), {} new-uncovered, {} CRAP regression(s)",
            count_lines(&verdict.regressions),
            count_lines(&verdict.new_uncovered),
            crap_regs.len(),
        );
        StepResult::fail("coverage").detail(detail)
    } else {
        let detail = format!(
            "clean — {} structural, {} improvement(s){}",
            count_lines(&verdict.structural),
            count_lines(&verdict.improvements),
            if healed { "; baselines healed" } else { "" },
        );
        StepResult::ok("coverage").detail(detail)
    };

    Ok((step, Some(report)))
}

fn count_lines(buckets: &[FileLines]) -> usize {
    buckets.iter().map(|b| b.lines.len()).sum()
}

fn normalize_json(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    Ok(v.to_string())
}

/// Canonical (key-sorted, via `serde_json::Value`'s `BTreeMap`) but
/// pretty-printed with a trailing newline — the on-disk form of the committed
/// manifest, so coverage diffs stay readable. Compared against [`normalize_json`]
/// output, which strips formatting, so the stored pretty form never triggers a
/// spurious rewrite.
fn pretty_json(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&v)?))
}

fn normalize_json_or_empty(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .map(|v| v.to_string())
        .unwrap_or_default()
}

fn git_repo_root() -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("running git rev-parse --show-toplevel")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_untracked_files() -> Result<Vec<String>> {
    let out = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .context("running git ls-files --others --exclude-standard")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

fn git_diff_unified0() -> Result<String> {
    // Pin the diff prefixes so a repo/CI with diff.noprefix, diff.mnemonicPrefix,
    // or color configured can't change the `+++ b/` prefix the parser keys on
    // (which would silently skip the file and mislabel edits as regressions).
    let out = Command::new("git")
        .args([
            "diff",
            "--unified=0",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "HEAD",
            "--",
        ])
        .output()
        .context("running git diff --unified=0 HEAD")?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::baseline::Baseline;

    fn fc(path: &str, lines: &[(u32, bool)]) -> FileCoverage {
        FileCoverage {
            path: path.into(),
            lines: lines
                .iter()
                .map(|(l, c)| LineCov {
                    line: *l,
                    covered: *c,
                    text: String::new(),
                })
                .collect(),
        }
    }

    fn verdict_with_improvement() -> CoverageVerdict {
        CoverageVerdict {
            improvements: vec![FileLines {
                file: "a.rs".into(),
                lines: vec![2],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn heals_a_shrunk_baseline_when_clean_in_fix_mode() {
        // Baseline accepted line 2 as a gap; current report shows line 2 now
        // covered (an improvement) and no failing delta → clean.
        let mut loaded = Baseline::default();
        loaded.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 2,
                text: String::new(),
            }],
        );
        let current = vec![fc("a.rs", &[(2, true)])];
        let verdict = verdict_with_improvement();

        let (new_baseline, healed) = heal_baseline(&verdict, &[], &current, &loaded, Mode::Fix);

        assert!(healed, "clean + improvement must heal in Fix mode");
        let b = new_baseline.expect("expected a healed baseline");
        // The improved gap is dropped — the new baseline has no gap for a.rs.
        assert!(b.gaps("a.rs").is_empty());
    }

    #[test]
    fn does_not_heal_when_a_regression_is_present_even_in_fix_mode() {
        let mut loaded = Baseline::default();
        loaded.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 2,
                text: String::new(),
            }],
        );
        let current = vec![fc("a.rs", &[(2, true), (5, false)])];
        let verdict = CoverageVerdict {
            regressions: vec![FileLines {
                file: "a.rs".into(),
                lines: vec![5],
            }],
            ..Default::default()
        };

        let (new_baseline, healed) = heal_baseline(&verdict, &[], &current, &loaded, Mode::Fix);

        assert!(!healed, "a regression must never heal");
        assert!(new_baseline.is_none());
    }

    #[test]
    fn does_not_heal_when_crap_regressions_present() {
        let loaded = Baseline::default();
        let current = vec![fc("a.rs", &[(1, true)])];
        let verdict = CoverageVerdict::default();
        let crap = vec![CrapRegression {
            file: "a.rs".into(),
            function: "f".into(),
            old: 2.0,
            new: 9.0,
        }];

        let (new_baseline, healed) = heal_baseline(&verdict, &crap, &current, &loaded, Mode::Fix);

        assert!(!healed);
        assert!(new_baseline.is_none());
    }

    #[test]
    fn never_heals_in_check_mode() {
        let mut loaded = Baseline::default();
        loaded.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 2,
                text: String::new(),
            }],
        );
        let current = vec![fc("a.rs", &[(2, true)])];
        let verdict = verdict_with_improvement();

        let (new_baseline, healed) = heal_baseline(&verdict, &[], &current, &loaded, Mode::Check);

        assert!(!healed, "Mode::Check must never heal");
        assert!(new_baseline.is_none());
    }
}

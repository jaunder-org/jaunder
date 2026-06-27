//! Coverage post-processing engine: parse the instrumented text report and the
//! CRAP report the Nix `coverage` check emits, classify each line-coverage delta
//! against the committed baseline, gate on regressions, and (in `Mode::Fix`)
//! auto-heal the shrink-only baseline + CRAP manifest.

use std::collections::{HashMap, HashSet};
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::coverage::diffmap::LineMap;
use crate::result::{Mode, StepResult};

pub mod baseline;
pub mod classify;
pub mod crap;
pub mod diffmap;
pub mod reanchor;
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
    /// Line-clean iff there is no line-identity failure: no regressions and no
    /// new uncovered lines. The gate itself keys on re-anchor safety
    /// (`reanchor::reanchor_is_safe`), not this; it remains a convenience for the
    /// classifier's own tests, which assert the per-bucket outcome.
    #[cfg(test)]
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
/// Heal happens ONLY when the run is a safe re-anchor: `Mode::Fix`, no genuine
/// coverage lowering (`safety.safe` — see [`reanchor`]), and no CRAP
/// regressions. The new baseline is simply `Baseline::from_files(&current)`:
/// "safe" means every current uncovered line is either an already-accepted gap
/// or the re-anchored image of one (identical text), so regenerating from the
/// current report drops improved (now-covered) and structural (deleted) gaps,
/// re-numbers survivors to working-tree lines, and never accepts a genuinely
/// new uncovered text. We only persist when it differs from the loaded baseline.
///
/// Returns `(None, false)` whenever anything fails or in `Mode::Check`.
fn heal_baseline(
    safety: &reanchor::ReanchorSafety,
    crap_regs: &[CrapRegression],
    current: &[FileCoverage],
    loaded: &Baseline,
    mode: Mode,
) -> (Option<Baseline>, bool) {
    let clean = safety.safe && crap_regs.is_empty();
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

    let anchor = baseline_anchor_commit()?;
    let diff = git_diff_anchor_to_worktree(&anchor)?;
    let mut maps = diffmap::parse_unified_diff(&diff);
    synthesize_untracked_maps(&mut maps, &current, &untracked_rs_files()?);

    let baseline = Baseline::load(BASELINE_PATH)?;
    let verdict = classify::classify(&current, &baseline, &maps);
    // Line-identity is only the first pass: a line-shifting change can flag
    // phantom regressions/new_uncovered. Re-anchor safety keys the gate on
    // uncovered-TEXT identity, so a pure move (removed-then-reappeared with the
    // same text) is recognised as safe rather than a lowering.
    let safety = reanchor::reanchor_is_safe(&verdict, &current, &baseline);

    let old_crap_manifest = std::fs::read_to_string(CRAP_MANIFEST_PATH).unwrap_or_default();
    let crap_regs = if old_crap_manifest.trim().is_empty() {
        Vec::new()
    } else {
        crap::compare(&crap_report_str, &old_crap_manifest)?
    };

    let gate_fails = !safety.safe || !crap_regs.is_empty();

    // Auto-heal — only on a safe re-anchor (Mode::Fix), never on a lowering.
    let (new_baseline, mut healed) = heal_baseline(&safety, &crap_regs, &current, &baseline, mode);
    if let Some(b) = &new_baseline {
        b.save(BASELINE_PATH)?;
    }
    // Likewise regenerate the CRAP manifest when there are no CRAP regressions
    // (Mode::Fix) and it differs from the committed manifest.
    if matches!(mode, Mode::Fix) && safety.safe && crap_regs.is_empty() {
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
            "{} coverage lowering(s), {} CRAP regression(s)",
            safety.lowering.len(),
            crap_regs.len(),
        );
        StepResult::fail("coverage").detail(detail)
    } else {
        // When safe, any line-flagged regression/new_uncovered was a pure move,
        // i.e. re-anchored rather than a real loss.
        let reanchored = count_lines(&verdict.regressions) + count_lines(&verdict.new_uncovered);
        let detail = format!(
            "clean — {reanchored} re-anchored, {} structural, {} improvement(s){}",
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

/// The commit the committed baseline was last healed at. The Nix report is built
/// from the working tree (== HEAD in CI), so the correct line map is
/// anchor→working-tree — NOT a HEAD-anchored diff (which ignored shifts
/// committed since the baseline, #11).
fn baseline_anchor_commit() -> Result<String> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%H", "--", BASELINE_PATH])
        .output()
        .context("running git log for baseline anchor")?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if sha.is_empty() {
        "HEAD".to_string()
    } else {
        sha
    })
}

/// `git diff` argv mapping the baseline anchor to the report's frame — the
/// WORKING TREE. A single commit arg (`<anchor>`, NOT `<anchor>..HEAD`) diffs the
/// anchor against the working tree. Pinned prefixes so repo/CI diff config can't
/// change the `+++ b/` prefix the parser keys on.
fn diff_args(anchor: &str) -> Vec<String> {
    vec![
        "diff".into(),
        "--unified=0".into(),
        "--no-color".into(),
        "--src-prefix=a/".into(),
        "--dst-prefix=b/".into(),
        anchor.to_string(),
        "--".into(),
    ]
}

fn git_diff_anchor_to_worktree(anchor: &str) -> Result<String> {
    let out = Command::new("git")
        .args(diff_args(anchor))
        .output()
        .context("running git diff <anchor> (anchor vs working tree)")?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Parse `git ls-files --others --exclude-standard -z` output: NUL-delimited
/// repo-root-relative paths. Empty entries (e.g. the trailing NUL) are dropped.
fn parse_untracked_list(stdout: &str) -> Vec<String> {
    stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Untracked, non-gitignored `.rs` files in the working tree (repo-root-relative).
/// The Nix coverage build instruments these (its source includes untracked,
/// non-gitignored files), but `git diff <anchor>` omits them — so they need a
/// synthesized all-added map. Thin git wrapper; the logic is in
/// `parse_untracked_list` / `synthesize_untracked_maps`.
fn untracked_rs_files() -> Result<Vec<String>> {
    let out = Command::new("git")
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "*.rs",
        ])
        .output()
        .context("running git ls-files --others (untracked .rs files)")?;
    Ok(parse_untracked_list(&String::from_utf8_lossy(&out.stdout)))
}

/// For each untracked file that actually appears in the coverage report, install
/// an all-added `LineMap` over its reported line numbers, so its uncovered lines
/// classify as `new_uncovered` instead of a phantom `regression`. Files already
/// carrying a diff-derived map (they appeared in the anchor diff) are left alone;
/// untracked files absent from the report (not compiled) are ignored.
fn synthesize_untracked_maps(
    maps: &mut HashMap<String, LineMap>,
    current: &[FileCoverage],
    untracked: &[String],
) {
    let untracked: HashSet<&str> = untracked.iter().map(String::as_str).collect();
    for f in current {
        if untracked.contains(f.path.as_str()) && !maps.contains_key(&f.path) {
            let lines = f.lines.iter().map(|l| l.line);
            maps.insert(f.path.clone(), LineMap::all_added(lines));
        }
    }
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

    #[test]
    fn diff_args_span_anchor_to_worktree_single_commit() {
        let args = diff_args("abc123");
        assert!(args.contains(&"abc123".to_string()));
        // A single commit arg = anchor-vs-working-tree; a `..HEAD` range would
        // wrongly ignore uncommitted edits (which ARE in the Nix report).
        assert!(!args.iter().any(|a| a.contains("..")));
    }

    #[test]
    fn parse_untracked_list_splits_nul_and_drops_empties() {
        assert_eq!(
            parse_untracked_list("a.rs\0server/src/b.rs\0"),
            vec!["a.rs".to_string(), "server/src/b.rs".to_string()]
        );
        assert!(parse_untracked_list("").is_empty());
    }

    #[test]
    fn synthesizes_all_added_map_for_untracked_file_in_report() {
        let current = vec![fc("server/src/new.rs", &[(1, true), (2, false)])];
        let mut maps: HashMap<String, LineMap> = HashMap::new();
        synthesize_untracked_maps(&mut maps, &current, &["server/src/new.rs".to_string()]);
        let m = maps.get("server/src/new.rs").expect("synthesized map");
        let mut added: Vec<u32> = m.added_lines().into_iter().collect();
        added.sort();
        assert_eq!(added, vec![1, 2]);
    }

    #[test]
    fn does_not_synthesize_for_a_file_not_in_the_untracked_list() {
        let current = vec![fc("tracked.rs", &[(1, false)])];
        let mut maps: HashMap<String, LineMap> = HashMap::new();
        synthesize_untracked_maps(&mut maps, &current, &[]);
        assert!(
            !maps.contains_key("tracked.rs"),
            "tracked file gets no synthesized map"
        );
    }

    #[test]
    fn does_not_overwrite_an_existing_diff_map() {
        let current = vec![fc("u.rs", &[(5, false)])];
        let mut maps: HashMap<String, LineMap> = HashMap::new();
        maps.insert("u.rs".to_string(), diffmap::empty_map()); // already mapped by the anchor diff
        synthesize_untracked_maps(&mut maps, &current, &["u.rs".to_string()]);
        assert!(
            maps.get("u.rs").unwrap().added_lines().is_empty(),
            "an existing diff map must be preserved, not replaced by all-added"
        );
    }

    #[test]
    fn untracked_uncovered_line_classifies_as_new_uncovered_not_regression() {
        // The end-to-end logic (minus the git shell): an untracked file's
        // uncovered line must be new_uncovered, never a phantom regression.
        let current = vec![fc("new.rs", &[(1, true), (2, false)])];
        let mut maps: HashMap<String, LineMap> = HashMap::new();
        synthesize_untracked_maps(&mut maps, &current, &["new.rs".to_string()]);
        let verdict = classify::classify(&current, &Baseline::default(), &maps);
        assert_eq!(
            verdict.new_uncovered,
            vec![FileLines {
                file: "new.rs".into(),
                lines: vec![2]
            }]
        );
        assert!(
            verdict.regressions.is_empty(),
            "untracked new code must not be a regression"
        );
    }

    #[test]
    fn crap_heal_is_idempotent_and_pretty() {
        // The heal writes pretty, key-sorted JSON and compares via the
        // formatting-independent `normalize_json`, so re-healing an already-healed
        // manifest is a no-op — no spurious multi-thousand-line diff churn (#7).
        let compact =
            r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let pretty = pretty_json(compact).unwrap();
        assert!(
            pretty.contains('\n'),
            "heal must write multi-line pretty JSON"
        );
        assert_eq!(
            normalize_json(&pretty).unwrap(),
            normalize_json(compact).unwrap(),
            "pretty and compact must normalize equal, so a re-heal does not rewrite"
        );
    }

    fn safe() -> reanchor::ReanchorSafety {
        reanchor::ReanchorSafety {
            safe: true,
            lowering: vec![],
        }
    }

    #[test]
    fn heals_a_shrunk_baseline_when_safe_in_fix_mode() {
        // Baseline accepted line 2 as a gap; current report shows line 2 now
        // covered (an improvement) and no genuine lowering → safe.
        let mut loaded = Baseline::default();
        loaded.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 2,
                text: String::new(),
            }],
        );
        let current = vec![fc("a.rs", &[(2, true)])];

        let (new_baseline, healed) = heal_baseline(&safe(), &[], &current, &loaded, Mode::Fix);

        assert!(healed, "safe + improvement must heal in Fix mode");
        let b = new_baseline.expect("expected a healed baseline");
        // The improved gap is dropped — the new baseline has no gap for a.rs.
        assert!(b.gaps("a.rs").is_empty());
    }

    #[test]
    fn does_not_heal_when_a_lowering_is_present_even_in_fix_mode() {
        let mut loaded = Baseline::default();
        loaded.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 2,
                text: String::new(),
            }],
        );
        let current = vec![fc("a.rs", &[(2, true), (5, false)])];
        let unsafe_ = reanchor::ReanchorSafety {
            safe: false,
            lowering: vec![reanchor::LineText {
                file: "a.rs".into(),
                line: 5,
                text: String::new(),
            }],
        };

        let (new_baseline, healed) = heal_baseline(&unsafe_, &[], &current, &loaded, Mode::Fix);

        assert!(!healed, "a genuine lowering must never heal");
        assert!(new_baseline.is_none());
    }

    #[test]
    fn does_not_heal_when_crap_regressions_present() {
        let loaded = Baseline::default();
        let current = vec![fc("a.rs", &[(1, true)])];
        let crap = vec![CrapRegression {
            file: "a.rs".into(),
            function: "f".into(),
            old: 2.0,
            new: 9.0,
        }];

        // Safe re-anchor but a CRAP regression independently blocks the heal.
        let (new_baseline, healed) = heal_baseline(&safe(), &crap, &current, &loaded, Mode::Fix);

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

        let (new_baseline, healed) = heal_baseline(&safe(), &[], &current, &loaded, Mode::Check);

        assert!(!healed, "Mode::Check must never heal");
        assert!(new_baseline.is_none());
    }

    #[test]
    fn safe_reanchor_heals_in_fix_and_not_in_check() {
        // A baseline gap re-anchors to a new line (same text); the heal
        // regenerates from current in Fix, and never mutates in Check.
        let mut loaded = Baseline::default();
        loaded.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 2,
                text: "x".into(),
            }],
        );
        let current = vec![FileCoverage {
            path: "a.rs".into(),
            lines: vec![LineCov {
                line: 9,
                covered: false,
                text: "x".into(),
            }],
        }];

        let (fix, healed) = heal_baseline(&safe(), &[], &current, &loaded, Mode::Fix);
        assert!(healed && fix.is_some(), "Fix re-anchors a safe drift");

        let (chk, healed) = heal_baseline(&safe(), &[], &current, &loaded, Mode::Check);
        assert!(!healed && chk.is_none(), "Check never mutates");
    }
}

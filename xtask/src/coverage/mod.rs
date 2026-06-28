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
    // Compare line-INDEPENDENTLY: a pure line-shift (same accepted-uncovered
    // texts, new line numbers) is NOT rewritten — the committed line numbers are
    // a hint, so the baseline doesn't churn on every shift and the pre-commit
    // gate's fail-and-restage fires only on genuine coverage changes (#113).
    // (Skipping leaves the anchor — the last commit that touched the file — in
    // place, so the anchor→worktree diff can span more history; that only grows
    // the diff, the reanchor text-multiset check stays sound regardless.)
    if healed.text_fingerprint() != loaded.text_fingerprint() {
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

    // Load the baseline from the anchor commit (NOT the working tree) so its
    // frame matches the diff's "from" frame even when a Fix-mode heal left an
    // uncommitted working-tree baseline — otherwise the next classify would
    // double-shift it and `validate` would contradict `check` (#110).
    let baseline = load_baseline_at_anchor(&anchor)?;
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
        // Compare line-independently so a pure line-shift is a no-op; write the
        // full pretty manifest (WITH line, the labelled hint) only when a
        // CRAP-relevant field actually changed. Writing pretty keeps coverage
        // diffs human-readable (a one-line blob makes them unreadable).
        let new_canon = normalize_crap_without_line(&crap_report_str)?;
        let old_canon = normalize_crap_without_line(&old_crap_manifest).unwrap_or_default();
        if new_canon != old_canon {
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
        StepResult::fail("coverage").detail(failure_report(&safety.lowering, &crap_regs))
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

/// Render a coverage-gate failure as a concise, actionable report: the exact
/// `file:line: text` of each genuine lowering and `file::fn old → new` of each
/// CRAP regression, plus what to do — so the invoker never has to diff the
/// baseline or read the raw report by hand (#87/#88). Capped so a large failure
/// stays one screen; the count and "… N more" make the truncation explicit.
fn failure_report(lowering: &[reanchor::LineText], crap_regs: &[CrapRegression]) -> String {
    use std::fmt::Write as _;
    const MAX: usize = 25;
    let mut s = format!(
        "{} coverage lowering(s), {} CRAP regression(s)",
        lowering.len(),
        crap_regs.len(),
    );
    if !lowering.is_empty() {
        s.push_str("\n  coverage dropped here:");
        for l in lowering.iter().take(MAX) {
            let _ = write!(s, "\n    {}:{}: {}", l.file, l.line, l.text.trim());
        }
        if lowering.len() > MAX {
            let _ = write!(s, "\n    … and {} more", lowering.len() - MAX);
        }
    }
    if !crap_regs.is_empty() {
        s.push_str("\n  CRAP worsened:");
        for c in crap_regs.iter().take(MAX) {
            let _ = write!(
                s,
                "\n    {}::{}  {:.2} → {:.2}",
                c.file, c.function, c.old, c.new
            );
        }
        if crap_regs.len() > MAX {
            let _ = write!(s, "\n    … and {} more", crap_regs.len() - MAX);
        }
    }
    // Truthful, category-split guidance. The gate already absorbs benign
    // line-shifts before failing, so a shown report is a real failure — don't
    // promise an auto-fix that can't apply here.
    if !lowering.is_empty() {
        s.push_str(
            "\n  → these lines are a real coverage loss unless the baseline is stale\
             \n    (a shift the anchor diff couldn't see, e.g. after a rebase): add a\
             \n    test, or re-anchor only after confirming they are not a genuine loss.",
        );
    }
    if !crap_regs.is_empty() {
        s.push_str(
            "\n  → CRAP: reduce the function's complexity or improve its coverage;\
             \n    refresh crap-manifest.json (with approval) only if it is stale drift.",
        );
    }
    s
}

/// Canonical, line- and order-independent form of a CRAP report: each entry
/// minus its `line`, with key-sorted JSON (serde_json `Value` is a `BTreeMap`),
/// and the entry set itself sorted. Two reports that differ only in line
/// attribution (a pure shift) normalize equal, so the Fix-mode heal does not
/// rewrite `crap-manifest.json` unless some non-`line` field changed — i.e. the
/// `crap` score or its `coverage`/`cyclomatic` inputs, or the set of functions
/// (#7). The `line` field is retained in the written manifest as a
/// non-authoritative jump-to hint that refreshes wholesale on the next such
/// change.
fn normalize_crap_without_line(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    let mut rows: Vec<String> = v
        .get("entries")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| {
                    let mut e = e.clone();
                    if let Some(o) = e.as_object_mut() {
                        o.remove("line");
                    }
                    e.to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort();
    Ok(rows.join("\n"))
}

/// Canonical (key-sorted, via `serde_json::Value`'s `BTreeMap`) but
/// pretty-printed with a trailing newline — the on-disk form of the committed
/// manifest, so coverage diffs stay readable.
fn pretty_json(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&v)?))
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

/// Load the accepted-uncovered baseline **from the anchor commit**, not the
/// working tree. The classifier maps baseline gap lines forward through
/// `git diff <anchor>..worktree`, so the baseline must be in the anchor's frame.
/// A working-tree baseline that `Mode::Fix` just *healed* but did not commit is
/// in the WORKTREE frame; loading it here would re-apply the diff and double-shift
/// every gap, so `validate` would contradict the `check` that produced it (#110).
/// Loading from the anchor keeps the baseline frame == the diff's "from" frame.
fn load_baseline_at_anchor(anchor: &str) -> Result<Baseline> {
    load_baseline_at_anchor_in(std::path::Path::new("."), anchor)
}

/// `repo`-parameterised core (for tests). Falls back to the working-tree file
/// only when the baseline does not exist at the anchor (never committed —
/// bootstrap/first run), preserving the pre-#110 behavior there.
fn load_baseline_at_anchor_in(repo: &std::path::Path, anchor: &str) -> Result<Baseline> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo);
    // Scrub the repo-redirecting env so `git show` targets `repo`, not a hook's
    // exported GIT_DIR (the git_at hazard — see CONTRIBUTING / xtask::lib).
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        cmd.env_remove(var);
    }
    let out = cmd
        .args(["show", &format!("{anchor}:{BASELINE_PATH}")])
        .output()
        .context("running git show <anchor>:coverage-baseline.json")?;
    if out.status.success() {
        Baseline::from_json(&String::from_utf8(out.stdout)?)
    } else {
        // Not present at the anchor (never committed) → first-run bootstrap:
        // fall back to the working-tree file (pre-#110 behavior).
        Baseline::load(BASELINE_PATH)
    }
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

    /// Run `git -C dir <args>`, scrubbed of repo-redirecting env (the git_at
    /// hazard — a bare `git` here would corrupt the real repo under a hook).
    fn git_in(dir: &std::path::Path, args: &[&str]) {
        let ok = scrubbed_git(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    fn scrubbed_git(dir: &std::path::Path) -> std::process::Command {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(dir);
        for v in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            cmd.env_remove(v);
        }
        cmd
    }

    #[test]
    fn loads_baseline_from_anchor_commit_not_working_tree() {
        // #110: after a Fix-mode heal writes a re-anchored (worktree-frame)
        // baseline without committing, the next classify must NOT load that dirty
        // working-tree baseline — it would double-shift it. load_baseline_at_anchor
        // returns the COMMITTED baseline at the anchor, ignoring the working tree.
        let tmp = std::env::temp_dir().join(format!("jaunder-anchortest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        git_in(&tmp, &["init", "-q"]);
        git_in(&tmp, &["config", "user.email", "t@t"]);
        git_in(&tmp, &["config", "user.name", "t"]);

        // Commit a baseline with a gap at line 5.
        let mut committed = Baseline::default();
        committed.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 5,
                text: "x".into(),
            }],
        );
        std::fs::write(tmp.join(BASELINE_PATH), committed.to_json()).unwrap();
        git_in(&tmp, &["add", BASELINE_PATH]);
        git_in(&tmp, &["commit", "-q", "-m", "baseline"]);
        let anchor = String::from_utf8(
            scrubbed_git(&tmp)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        // Simulate the heal: overwrite the WORKING-TREE baseline with a different
        // (worktree-frame) line number, uncommitted.
        let mut healed = Baseline::default();
        healed.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 99,
                text: "x".into(),
            }],
        );
        std::fs::write(tmp.join(BASELINE_PATH), healed.to_json()).unwrap();

        let loaded = load_baseline_at_anchor_in(&tmp, &anchor).unwrap();
        assert_eq!(
            loaded.gaps("a.rs"),
            committed.gaps("a.rs"),
            "must load the committed (anchor) baseline, not the healed working tree"
        );

        let _ = std::fs::remove_dir_all(&tmp);
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
    fn crap_normalize_ignores_line_and_formatting() {
        // Same scores, different line attribution + key order + whitespace →
        // equal canonical form, so the heal does not rewrite the manifest (#7).
        let a = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let b = r#"{ "entries": [ {"crap":2.0,"function":"f","file":"a.rs","crate":"c","line":888} ] }"#;
        assert_eq!(
            normalize_crap_without_line(a).unwrap(),
            normalize_crap_without_line(b).unwrap(),
            "line + key order + whitespace must not affect the canonical form"
        );
    }

    #[test]
    fn crap_normalize_detects_a_score_change() {
        let a = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let c = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":9.0}]}"#;
        assert_ne!(
            normalize_crap_without_line(a).unwrap(),
            normalize_crap_without_line(c).unwrap(),
            "a real CRAP change must change the canonical form"
        );
    }

    #[test]
    fn crap_pretty_json_is_multiline() {
        let compact =
            r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        assert!(pretty_json(compact).unwrap().contains('\n'));
    }

    #[test]
    fn failure_report_lists_lines_crap_and_recovery() {
        let lowering = vec![reanchor::LineText {
            file: "a.rs".into(),
            line: 10,
            text: "    let x = bar()?;".into(),
        }];
        let crap = vec![CrapRegression {
            file: "b.rs".into(),
            function: "f".into(),
            old: 9.0,
            new: 11.0,
        }];
        let r = failure_report(&lowering, &crap);
        assert!(r.contains("1 coverage lowering(s), 1 CRAP regression(s)"));
        assert!(r.contains("a.rs:10: let x = bar()?;"), "{r}"); // text trimmed
        assert!(r.contains("b.rs::f  9.00 → 11.00"), "{r}");
        assert!(r.contains("real coverage loss"), "lowering guidance: {r}");
        assert!(r.contains("CRAP: reduce"), "crap guidance: {r}");
    }

    #[test]
    fn failure_report_guidance_is_category_conditional() {
        // CRAP-only failure must not show the coverage-lowering guidance, and
        // must never claim `cargo xtask check` re-anchors it (it can't here).
        let crap = vec![CrapRegression {
            file: "b.rs".into(),
            function: "f".into(),
            old: 9.0,
            new: 11.0,
        }];
        let r = failure_report(&[], &crap);
        assert!(!r.contains("coverage dropped here"), "{r}");
        assert!(!r.contains("real coverage loss"), "{r}");
        assert!(!r.contains("cargo xtask check"), "no false auto-fix: {r}");
        assert!(r.contains("CRAP: reduce"), "{r}");
    }

    #[test]
    fn failure_report_caps_long_lists() {
        let lowering: Vec<_> = (0..30)
            .map(|i| reanchor::LineText {
                file: "a.rs".into(),
                line: i,
                text: "x".into(),
            })
            .collect();
        let r = failure_report(&lowering, &[]);
        assert!(r.contains("30 coverage lowering(s)"));
        assert!(r.contains("… and 5 more"), "{r}"); // 30 - cap 25
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
    fn fix_skips_pure_line_shift_but_heals_a_real_change() {
        // line is a hint (#113): a pure line-shift (gap "x" 2→9, same text) must
        // NOT rewrite, so the pre-commit fail-and-restage doesn't fire on benign
        // shifts. A genuine coverage change (gap covered) still heals.
        let mut loaded = Baseline::default();
        loaded.set_gaps(
            "a.rs",
            vec![baseline::Gap {
                line: 2,
                text: "x".into(),
            }],
        );

        // Pure line-shift: "x" moved to line 9, still uncovered.
        let shifted = vec![FileCoverage {
            path: "a.rs".into(),
            lines: vec![LineCov {
                line: 9,
                covered: false,
                text: "x".into(),
            }],
        }];
        let (b, healed) = heal_baseline(&safe(), &[], &shifted, &loaded, Mode::Fix);
        assert!(
            !healed && b.is_none(),
            "a pure line-shift must not rewrite (line is a hint)"
        );

        // Real change: "x" is now covered → gap drops (texts differ) → heal.
        let covered = vec![FileCoverage {
            path: "a.rs".into(),
            lines: vec![LineCov {
                line: 9,
                covered: true,
                text: "x".into(),
            }],
        }];
        let (b, healed) = heal_baseline(&safe(), &[], &covered, &loaded, Mode::Fix);
        assert!(healed && b.is_some(), "a real coverage change still heals");

        // Check never mutates, even on a real change.
        let (chk, healed) = heal_baseline(&safe(), &[], &covered, &loaded, Mode::Check);
        assert!(!healed && chk.is_none(), "Check never mutates");
    }
}

//! On-demand drift guard for the Nix `coverage` derivation's source filter (#241).
//!
//! #231 bounded the `coverage` derivation's `src` to cargo sources (+ an explicit
//! `csr/index.html`), closing the #37 impurity. This module guards that filter
//! against *silent* drift: [`probe_verdict`] asserts the two contract invariants over
//! three measured `coverage.drvPath` values —
//!
//! - adding a filter-**excluded** file must NOT change the drvPath (else the filter
//!   re-admits junk → the #37 impurity returns), and
//! - adding an **instrumented** `.rs` MUST change the drvPath (else the filter drops
//!   source → a coverage hole the stateless gate can never see).
//!
//! The pure verdict lives here; the impure orchestration (an ephemeral worktree that
//! stages each probe file and evaluates its drvPath) is [`probe_source`]. See the
//! spec for the load-bearing subtlety: nix ignores *untracked* new files even on a
//! dirty tree, so probe files must be `git add`-ed to be measured.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::git;
use crate::result::StepResult;
use crate::steps::nix::eval_coverage_drvpath;

/// The two ways the coverage `src` filter can drift — each a distinct contract break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftError {
    /// A filter-excluded file changed `coverage.drvPath`: the filter now admits junk
    /// (the #37 impurity regressed).
    AdmitsJunk { base: String, junk: String },
    /// An instrumented `.rs` did NOT change `coverage.drvPath`: the filter drops
    /// source, so those lines are never measured (a coverage hole).
    DropsSource { base: String },
}

impl fmt::Display for DriftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriftError::AdmitsJunk { base, junk } => write!(
                f,
                "coverage src filter admits junk: staging an excluded file changed \
                 coverage.drvPath ({base} -> {junk}) — the #37 impurity regressed"
            ),
            DriftError::DropsSource { base } => write!(
                f,
                "coverage src filter drops source: staging an instrumented .rs left \
                 coverage.drvPath unchanged ({base}) — those lines would never be measured"
            ),
        }
    }
}

impl std::error::Error for DriftError {}

/// Assert the coverage `src` filter's two invariants over three measured drvPaths:
/// `base` (clean HEAD), `junk` (base + a staged filter-excluded file), and `rs`
/// (base + a staged instrumented `.rs`). Impurity (admits-junk) is checked before the
/// coverage hole (drops-source) so the more severe regression is reported first.
pub fn probe_verdict(base: &str, junk: &str, rs: &str) -> Result<(), DriftError> {
    if junk != base {
        return Err(DriftError::AdmitsJunk {
            base: base.to_owned(),
            junk: junk.to_owned(),
        });
    }
    if rs == base {
        return Err(DriftError::DropsSource {
            base: base.to_owned(),
        });
    }
    Ok(())
}

/// Removes the ephemeral probe worktree on every exit path (return, error, panic).
/// The whole point of an RAII guard here is the panic path: a bare cleanup call at
/// the end of `run_probe` would leak the worktree if any `?` bailed or a panic
/// unwound through it.
struct WorktreeGuard {
    repo_root: PathBuf,
    path: PathBuf,
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        let _ = git::at(&self.repo_root)
            .args(["-c", "core.hooksPath="])
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .status();
    }
}

/// Run a git subcommand in `dir` with hooks disabled; bail on a non-zero exit.
/// Hooks are disabled defensively — `worktree add` can fire a `post-checkout`
/// hook, and we never want the repo's gate hooks running inside the probe. The
/// `-c core.hooksPath=` prefix is the only probe-specific bit; the run-and-check
/// plumbing lives in [`git::run`].
fn git_run(dir: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["-c", "core.hooksPath="];
    full.extend_from_slice(args);
    git::run(dir, &full)
}

/// The user-facing step: measure the three coverage drvPaths and apply
/// [`probe_verdict`]. Any I/O failure (nix/git) or a drift verdict becomes a failing
/// [`StepResult`] whose detail names the broken invariant.
pub fn probe_source() -> StepResult {
    match run_probe() {
        Ok(()) => StepResult::ok("coverage-probe-source")
            .detail("coverage src filter contract holds (junk excluded, source measured)"),
        Err(e) => StepResult::fail("coverage-probe-source").detail(format!("{e:#}")),
    }
}

/// Measure `coverage.drvPath` across three tree states in an ephemeral worktree and
/// return the verdict. The worktree is checked out at `HEAD`, so the probe guards the
/// *committed* filter (what CI/PRs carry), not local uncommitted edits. Probe files
/// are `git add`-ed, not left untracked — nix ignores untracked new files even on a
/// dirty tree (see the module docs / spec).
fn run_probe() -> Result<()> {
    let repo_root = std::env::current_dir().context("resolving cwd")?;
    let tmp = repo_root.join(".xtask/coverage-probe.worktree");
    fs::create_dir_all(repo_root.join(".xtask")).context("creating .xtask")?;
    // Clear any leftover from a prior crash (ignore failure — usually nothing there).
    // Silence output: a "not a working tree" fatal is the normal no-leftover case and
    // would be misleading noise in the CI log.
    let _ = git::at(&repo_root)
        .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
        .arg(&tmp)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let tmp_str = tmp.to_str().context("worktree path is not UTF-8")?;
    git_run(
        &repo_root,
        &["worktree", "add", "--detach", tmp_str, "HEAD"],
    )?;
    let _guard = WorktreeGuard {
        repo_root: repo_root.clone(),
        path: tmp.clone(),
    };

    // Dirty an EXCLUDED tracked file so *every* eval runs against a dirty tree.
    // Rationale (verified against a shallow clone): on a CLEAN worktree nix's flake
    // git-fetcher walks history (revCount) to resolve the rev, which fails on CI's
    // shallow checkout — the PR is a merge commit whose parents are grafted away
    // ("getting Git object <parent>: object not found"). A dirty tree makes nix copy
    // the working dir and read only HEAD (present), skipping the parent walk.
    // README.md is filter-excluded (`.md`), so the dirtying is constant and never
    // perturbs the coverage drvPath. (This is orthogonal to the `git add` staging,
    // which exists so the *new* probe files are visible at all.)
    let readme = tmp.join("README.md");
    let mut readme_bytes = fs::read(&readme).context("reading README.md to dirty it")?;
    readme_bytes.push(b'\n');
    fs::write(&readme, readme_bytes).context("dirtying README.md")?;

    // State A: base (dirty tree, no probe files staged).
    let base = eval_coverage_drvpath(&tmp)?;

    // State B: staged junk (filter-excluded) → drvPath must be unchanged.
    fs::write(tmp.join("probe.txt"), b"").context("writing probe.txt")?;
    git_run(&tmp, &["add", "probe.txt"])?;
    let junk = eval_coverage_drvpath(&tmp)?;
    git_run(&tmp, &["rm", "--cached", "--quiet", "probe.txt"])?;
    fs::remove_file(tmp.join("probe.txt")).context("removing probe.txt")?;

    // State C: staged instrumented `.rs` → drvPath must change.
    let rs_rel = "server/src/__drift_probe.rs";
    fs::write(
        tmp.join(rs_rel),
        b"// coverage source-drift probe (#241); never committed.\n",
    )
    .context("writing probe .rs")?;
    git_run(&tmp, &["add", rs_rel])?;
    let rs = eval_coverage_drvpath(&tmp)?;

    // `DriftError: std::error::Error`, so `?` lifts it into `anyhow::Error`.
    probe_verdict(&base, &junk, &rs)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_run_succeeds_and_fails() {
        let dir = std::env::temp_dir().join(format!("jaunder-probe-gitrun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(git::at(&dir)
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success());
        assert!(git_run(&dir, &["status", "--porcelain"]).is_ok());
        assert!(git_run(&dir, &["mv", "nope", "nowhere"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn holds_when_junk_excluded_and_source_measured() {
        assert_eq!(probe_verdict("d-base", "d-base", "d-rs"), Ok(()));
    }

    #[test]
    fn admits_junk_when_junk_moves_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-rs"),
            Err(DriftError::AdmitsJunk {
                base: "d-base".into(),
                junk: "d-JUNKMOVED".into()
            })
        );
    }

    #[test]
    fn drops_source_when_rs_does_not_move_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-base", "d-base"),
            Err(DriftError::DropsSource {
                base: "d-base".into()
            })
        );
    }

    #[test]
    fn admits_junk_takes_precedence_over_drops_source() {
        // Both broken: junk moved AND rs == base. Junk (impurity) is checked first.
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-base"),
            Err(DriftError::AdmitsJunk {
                base: "d-base".into(),
                junk: "d-JUNKMOVED".into()
            })
        );
    }

    #[test]
    fn display_names_the_broken_invariant() {
        let j = DriftError::AdmitsJunk {
            base: "b".into(),
            junk: "j".into(),
        };
        assert!(j.to_string().contains("admits") && j.to_string().contains("junk"));
        let s = DriftError::DropsSource { base: "b".into() };
        assert!(s.to_string().contains("drops") && s.to_string().contains("source"));
    }
}

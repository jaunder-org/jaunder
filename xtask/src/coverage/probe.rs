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
use std::io::Write;
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
type WorktreeRemover<'a> =
    Box<dyn Fn(&Path, &Path) -> std::io::Result<std::process::ExitStatus> + 'a>;

struct WorktreeGuard<'a> {
    repo_root: PathBuf,
    path: PathBuf,
    remove: WorktreeRemover<'a>,
    stderr: Box<dyn Write + 'a>,
}

impl Drop for WorktreeGuard<'_> {
    fn drop(&mut self) {
        let status = (self.remove)(&self.repo_root, &self.path);
        report_worktree_cleanup(status, &mut self.stderr);
    }
}

fn report_worktree_cleanup(
    status: std::io::Result<std::process::ExitStatus>,
    stderr: &mut impl Write,
) {
    let failed = match status {
        Ok(status) => !status.success(),
        Err(_) => true,
    };
    if failed {
        let _ = writeln!(
            stderr,
            "xtask: warning: xtask.coverage.probe_worktree_cleanup: ignored failure while removing probe worktree"
        );
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
fn worktree_registered_with(tmp: &Path, query: impl FnOnce() -> Result<Vec<u8>>) -> Result<bool> {
    let tmp = tmp.to_str().context("worktree path is not UTF-8")?;
    let fields = query()?;
    Ok(fields.split(|byte| *byte == 0).any(|field| {
        std::str::from_utf8(field)
            .ok()
            .and_then(|field| field.strip_prefix("worktree "))
            == Some(tmp)
    }))
}

fn remove_registered_worktree_with(
    registered: bool,
    remove: impl FnOnce() -> std::io::Result<std::process::ExitStatus>,
) -> Result<()> {
    if !registered {
        return Ok(());
    }
    let status = remove().context("removing registered stale coverage-probe worktree")?;
    if !status.success() {
        anyhow::bail!("removing registered stale coverage-probe worktree failed with {status}");
    }
    Ok(())
}

fn run_probe() -> Result<()> {
    let repo_root = std::env::current_dir().context("resolving cwd")?;
    let tmp = repo_root.join(".xtask/coverage-probe.worktree");
    fs::create_dir_all(repo_root.join(".xtask")).context("creating .xtask")?;
    let registered = worktree_registered_with(&tmp, || {
        let output = git::at(&repo_root)
            .args(["worktree", "list", "--porcelain", "-z"])
            .output()
            .context("querying registered Git worktrees")?;
        if !output.status.success() {
            anyhow::bail!(
                "querying registered Git worktrees failed with {}",
                output.status
            );
        }
        Ok(output.stdout)
    })?;
    remove_registered_worktree_with(registered, || {
        git::at(&repo_root)
            .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    })?;

    let tmp_str = tmp.to_str().context("worktree path is not UTF-8")?;
    git_run(
        &repo_root,
        &["worktree", "add", "--detach", tmp_str, "HEAD"],
    )?;
    let _guard = WorktreeGuard {
        repo_root: repo_root.clone(),
        path: tmp.clone(),
        remove: Box::new(|repo_root, path| {
            git::at(repo_root)
                .args(["-c", "core.hooksPath="])
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        }),
        stderr: Box::new(std::io::stderr()),
    };

    // Dirty an EXCLUDED tracked file so *every* eval runs against a dirty tree:
    // a clean tree makes nix's flake fetcher walk grafted-away history on CI's
    // shallow checkout and fail
    // (docs/adr/0116-coverage-probe-dirty-tree-workaround.md).
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
        assert!(
            git::at(&dir)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
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
    fn stale_cleanup_uses_git_registration_not_directory_presence() {
        use std::os::unix::process::ExitStatusExt;

        let tmp = Path::new("/repo/.xtask/coverage-probe.worktree");
        let registered = worktree_registered_with(tmp, || {
            Ok(b"worktree /repo/.xtask/coverage-probe.worktree\0HEAD abc\0".to_vec())
        })
        .unwrap();
        assert!(
            registered,
            "a vanished directory can remain registered by Git"
        );

        let removed = std::cell::Cell::new(false);
        remove_registered_worktree_with(registered, || {
            removed.set(true);
            Ok(std::process::ExitStatus::from_raw(0))
        })
        .unwrap();
        assert!(removed.get());

        let absent = worktree_registered_with(tmp, || {
            Ok(b"worktree /repo/another-worktree\0HEAD def\0".to_vec())
        })
        .unwrap();
        remove_registered_worktree_with(absent, || {
            unreachable!("confirmed unregistered paths require no removal")
        })
        .unwrap();
    }

    #[test]
    fn stale_worktree_registry_query_failure_is_typed() {
        let error =
            worktree_registered_with(Path::new("/repo/.xtask/coverage-probe.worktree"), || {
                Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected registry failure",
                )))
            })
            .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
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

    #[test]
    fn ancillary_warning_probe_worktree_cleanup_preserves_verdict() {
        let verdict = probe_verdict("d-base", "d-base", "d-rs");
        let mut stderr = Vec::new();
        {
            let guard = WorktreeGuard {
                repo_root: PathBuf::from("/repo"),
                path: PathBuf::from("/repo/worktree"),
                remove: Box::new(|_, _| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "sensitive injected path",
                    ))
                }),
                stderr: Box::new(&mut stderr),
            };
            drop(guard);
        }
        assert_eq!(verdict, Ok(()));
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(
            warning
                .matches("xtask.coverage.probe_worktree_cleanup")
                .count(),
            1
        );
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
    }
}

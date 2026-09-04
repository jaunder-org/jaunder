//! Eval-only drift guard for the Nix invalidation boundaries from #1289.
//!
//! The pure comparison is intentionally independent from Nix and Git. The command
//! measures each tracked perturbation in a disposable worktree, then compares only
//! derivation identities; it never builds or realizes an output.

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::git;
use crate::result::StepResult;
use crate::steps::nix::eval_source_probe_drvpaths;

const WORKTREE_DIR: &str = ".xtask/nix-source-probe.worktree";
const BOUNDARY_NAMES: [&str; 4] = ["static-docs", "static-code", "site", "wasm-tests"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    Docs,
    Server,
    Web,
    Common,
    Macros,
}

impl Arm {
    const ALL: [Self; 5] = [
        Self::Docs,
        Self::Server,
        Self::Web,
        Self::Common,
        Self::Macros,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Docs => "docs",
            Self::Server => "server",
            Self::Web => "web",
            Self::Common => "common",
            Self::Macros => "macros",
        }
    }

    const fn marker_path(self) -> &'static str {
        match self {
            Self::Docs => "docs/__nix_source_probe.md",
            Self::Server => "server/src/__nix_source_probe.rs",
            Self::Web => "web/src/__nix_source_probe.rs",
            Self::Common => "common/src/__nix_source_probe.rs",
            Self::Macros => "macros/src/__nix_source_probe.rs",
        }
    }

    const fn expected_changes(self) -> [bool; 4] {
        match self {
            Self::Docs => [true, false, false, false],
            Self::Server => [false, true, false, false],
            Self::Web => [false, true, true, false],
            Self::Common | Self::Macros => [false, true, true, true],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrvPaths([String; 4]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    MissingChange {
        arm: Arm,
        boundary: &'static str,
        drv: String,
    },
    UnexpectedChange {
        arm: Arm,
        boundary: &'static str,
        base: String,
        changed: String,
    },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingChange { arm, boundary, drv } => write!(
                f,
                "{}/{}: expected derivation identity to change, but it remained {drv}",
                arm.name(),
                boundary
            ),
            Self::UnexpectedChange {
                arm,
                boundary,
                base,
                changed,
            } => write!(
                f,
                "{}/{}: expected derivation identity to remain {base}, but it changed to {changed}",
                arm.name(),
                boundary
            ),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Compare one perturbation against the exact boundary matrix. Check required
/// fan-out before over-inclusion so a dropped source is reported first.
pub fn compare_arm(base: &DrvPaths, arm: Arm, changed: &DrvPaths) -> Result<(), ProbeError> {
    for (index, expected_change) in arm.expected_changes().into_iter().enumerate() {
        if expected_change && changed.0[index] == base.0[index] {
            return Err(ProbeError::MissingChange {
                arm,
                boundary: BOUNDARY_NAMES[index],
                drv: base.0[index].clone(),
            });
        }
    }
    for (index, expected_change) in arm.expected_changes().into_iter().enumerate() {
        if !expected_change && changed.0[index] != base.0[index] {
            return Err(ProbeError::UnexpectedChange {
                arm,
                boundary: BOUNDARY_NAMES[index],
                base: base.0[index].clone(),
                changed: changed.0[index].clone(),
            });
        }
    }
    Ok(())
}

pub fn probe_source() -> StepResult {
    match run_probe() {
        Ok(()) => StepResult::ok("nix-probe-source")
            .detail("Nix invalidation boundary contract holds (docs/server/web/common/macros)"),
        Err(error) => StepResult::fail("nix-probe-source").detail(format!("{error:#}")),
    }
}

fn parse_drv_path(boundary: &str, output: &str) -> Result<String> {
    let path = output.trim();
    if path.is_empty() || !path.starts_with("/nix/store/") || !path.ends_with(".drv") {
        bail!("{boundary}: malformed nix eval drvPath output: {output:?}");
    }
    Ok(path.to_owned())
}

fn eval_paths_with(evaluate: impl FnOnce() -> Result<[String; 4]>) -> Result<DrvPaths> {
    let [docs, code, site, wasm] = evaluate()?;
    Ok(DrvPaths([
        parse_drv_path(BOUNDARY_NAMES[0], &docs)?,
        parse_drv_path(BOUNDARY_NAMES[1], &code)?,
        parse_drv_path(BOUNDARY_NAMES[2], &site)?,
        parse_drv_path(BOUNDARY_NAMES[3], &wasm)?,
    ]))
}

fn eval_paths(dir: &Path) -> Result<DrvPaths> {
    eval_paths_with(|| eval_source_probe_drvpaths(dir))
}

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
        match (self.remove)(&self.repo_root, &self.path) {
            Ok(status) if status.success() => {}
            _ => {
                let _ = writeln!(
                    self.stderr,
                    "xtask: warning: xtask.nix.probe_worktree_cleanup: ignored failure while removing probe worktree"
                );
            }
        }
    }
}

fn git_run(dir: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["-c", "core.hooksPath="];
    full.extend_from_slice(args);
    git::run(dir, &full)
}

fn worktree_registered(repo_root: &Path, path: &Path) -> Result<bool> {
    let output = git::at(repo_root)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()
        .context("querying registered Git worktrees")?;
    if !output.status.success() {
        bail!(
            "querying registered Git worktrees failed with {}",
            output.status
        );
    }
    let path = path.to_str().context("worktree path is not UTF-8")?;
    Ok(output.stdout.split(|byte| *byte == 0).any(|field| {
        std::str::from_utf8(field)
            .ok()
            .and_then(|field| field.strip_prefix("worktree "))
            == Some(path)
    }))
}

fn run_probe() -> Result<()> {
    let repo_root = std::env::current_dir().context("resolving cwd")?;
    let tmp = repo_root.join(WORKTREE_DIR);
    fs::create_dir_all(repo_root.join(".xtask")).context("creating .xtask")?;
    if worktree_registered(&repo_root, &tmp)? {
        let status = git::at(&repo_root)
            .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
            .arg(&tmp)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("removing stale nix-source-probe worktree")?;
        if !status.success() {
            bail!("removing stale nix-source-probe worktree failed with {status}");
        }
    }

    let tmp_str = tmp.to_str().context("worktree path is not UTF-8")?;
    git_run(
        &repo_root,
        &["worktree", "add", "--detach", tmp_str, "HEAD"],
    )?;
    let _guard = WorktreeGuard {
        repo_root: repo_root.clone(),
        path: tmp.clone(),
        remove: Box::new(|root, path| {
            git::at(root)
                .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
                .arg(path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        }),
        stderr: Box::new(std::io::stderr()),
    };

    // Every state is dirty. This keeps Nix's flake fetcher off grafted-away
    // history on shallow CI checkouts, while the shared change cancels out.
    let readme = tmp.join("README.md");
    let mut readme_bytes = fs::read(&readme).context("reading README.md to dirty it")?;
    readme_bytes.push(b'\n');
    fs::write(&readme, readme_bytes).context("dirtying README.md")?;

    let baseline = eval_paths(&tmp).context("evaluating baseline derivation identities")?;
    for arm in Arm::ALL {
        let marker = tmp.join(arm.marker_path());
        fs::write(&marker, format!("// nix source probe: {}\n", arm.name()))
            .with_context(|| format!("writing {} marker", arm.name()))?;
        git_run(&tmp, &["add", arm.marker_path()])?;
        let changed = eval_paths(&tmp).with_context(|| {
            format!(
                "evaluating {} perturbation derivation identities",
                arm.name()
            )
        })?;
        compare_arm(&baseline, arm, &changed)
            .with_context(|| format!("checking {} perturbation", arm.name()))?;
        git_run(&tmp, &["rm", "--cached", "--quiet", arm.marker_path()])?;
        fs::remove_file(marker).with_context(|| format!("removing {} marker", arm.name()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: [&str; 4]) -> DrvPaths {
        DrvPaths(names.map(str::to_owned))
    }

    #[test]
    fn matrix_accepts_exact_expected_changes() {
        let base = paths(["docs", "code", "site", "wasm"]);
        for (arm, changed) in [
            (Arm::Docs, paths(["docs-2", "code", "site", "wasm"])),
            (Arm::Server, paths(["docs", "code-2", "site", "wasm"])),
            (Arm::Web, paths(["docs", "code-2", "site-2", "wasm"])),
            (Arm::Common, paths(["docs", "code-2", "site-2", "wasm-2"])),
            (Arm::Macros, paths(["docs", "code-2", "site-2", "wasm-2"])),
        ] {
            assert_eq!(compare_arm(&base, arm, &changed), Ok(()));
        }
    }

    #[test]
    fn matrix_rejects_missing_required_fan_out() {
        let base = paths(["docs", "code", "site", "wasm"]);
        assert!(matches!(
            compare_arm(
                &base,
                Arm::Common,
                &paths(["docs", "code", "site-2", "wasm-2"])
            ),
            Err(ProbeError::MissingChange {
                boundary: "static-code",
                ..
            })
        ));
    }

    #[test]
    fn matrix_rejects_over_inclusion() {
        let base = paths(["docs", "code", "site", "wasm"]);
        assert!(matches!(
            compare_arm(
                &base,
                Arm::Server,
                &paths(["docs", "code-2", "site-2", "wasm"])
            ),
            Err(ProbeError::UnexpectedChange {
                boundary: "site",
                ..
            })
        ));
    }

    #[test]
    fn rejects_malformed_evaluation_output() {
        assert!(parse_drv_path("site", "not-a-derivation").is_err());
        assert!(parse_drv_path("site", "").is_err());
    }

    #[test]
    fn propagates_evaluation_failure() {
        let error = eval_paths_with(|| anyhow::bail!("nix eval failed")).unwrap_err();
        assert!(format!("{error:#}").contains("nix eval failed"));
    }
}

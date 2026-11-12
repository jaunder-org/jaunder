//! Admit the live checkout before host orchestration can mutate its state.
//!
//! Git supplies nonignored untracked paths; the selected flake's source predicate
//! decides which belong to the executable closure. No parallel file inventory or
//! hash protocol lives here. Native Cargo diagnostics do not omit untracked inputs.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::git;

pub const ADMIT_INPUTS: &str = r#"sourceAdmits:
let candidates = builtins.fromJSON (builtins.getEnv "JAUNDER_XTASK_UNTRACKED_INPUTS");
in map (candidate: candidate.path)
  (builtins.filter (candidate: sourceAdmits candidate.path candidate.kind) candidates)"#;

#[derive(Serialize)]
pub struct Candidate {
    pub path: String,
    pub kind: &'static str,
}

pub fn admit_untracked_inputs(root: &Path, source_admits: &str) -> Result<()> {
    let output = git::at(root)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
        .context("listing untracked checkout inputs")?;
    if !output.status.success() {
        bail!(
            "listing untracked inputs failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut candidates = Vec::new();
    for path in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(path).context("untracked input path is not UTF-8")?;
        let metadata = std::fs::symlink_metadata(root.join(path))
            .with_context(|| format!("reading untracked input {path}"))?;
        let kind = if metadata.is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "regular"
        } else {
            bail!("unsupported untracked input type: {path}");
        };
        candidates.push(Candidate {
            path: path.to_owned(),
            kind,
        });
    }
    if candidates.is_empty() {
        return Ok(());
    }
    let output = Command::new("nix")
        .args([
            "eval",
            "--impure",
            "--json",
            "--no-update-lock-file",
            "--no-write-lock-file",
        ])
        // Match the launcher without interpolating paths into installable syntax.
        .arg(source_admits)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .args(["--apply", ADMIT_INPUTS])
        .env(
            "JAUNDER_XTASK_UNTRACKED_INPUTS",
            serde_json::to_string(&candidates)?,
        )
        .output()
        .context("resolving declared Nix tooling inputs")?;
    if !output.status.success() {
        bail!(
            "cannot verify Nix tooling inputs; ensure new tooling files are tracked with git add (or git add -N): {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let omitted: Vec<String> =
        serde_json::from_slice(&output.stdout).context("decoding Nix tooling input admission")?;
    if !omitted.is_empty() {
        bail!(
            "Git flakes omit untracked declared inputs: {}. Track them with git add (or git add -N) before invoking the selected tool",
            omitted.join(", ")
        );
    }
    Ok(())
}

/// Select the caller's checkout, never the executable's Nix build directory.
/// Admission precedes hook installation, leases and command-owned result files.
pub fn prepare_checkout(marker: &str, source_admits: Option<&str>) -> Result<()> {
    let start = std::env::current_dir().context("resolving caller directory")?;
    let root = std::path::PathBuf::from(git::toplevel(&start)?);
    for marker in ["flake.nix", marker] {
        if !std::fs::metadata(root.join(marker))
            .with_context(|| format!("reading Jaunder checkout marker {marker}"))?
            .is_file()
        {
            bail!("not a Jaunder checkout: {marker} is not a file");
        }
    }
    if let Some(source_admits) = source_admits {
        admit_untracked_inputs(&root, source_admits)?;
    }
    std::env::set_current_dir(root).context("selecting caller checkout")
}

/// Ancillary hook configuration cannot prevent the admitted policy from running.
pub fn ensure_hooks_installed(owner: &str) -> bool {
    match git::ensure_hooks_path(Path::new(".")) {
        Ok(true) => {
            eprintln!("{owner}: set core.hooksPath = {}", git::HOOKS_PATH);
            true
        }
        Ok(false) => true,
        Err(error) => {
            eprintln!("{owner}: warning: could not set core.hooksPath: {error:#}");
            false
        }
    }
}

/// Native Cargo reads live source; Git-backed Nix outputs additionally refuse omitted inputs.
pub fn prepare_policy_checkout(owner: &str) -> Result<()> {
    prepare_checkout(
        "tools/Cargo.toml",
        option_env!("JAUNDER_REPOSITORY_POLICY_SOURCE"),
    )?;
    ensure_hooks_installed(owner);
    Ok(())
}

//! Serial Nix evaluation of the derivation identities `nix probe-source` guards.
use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

/// Run `nix eval --raw --accept-flake-config <installable>`, optionally in a
/// supplied flake directory.
fn nix_eval_raw(dir: Option<&Path>, installable: &str) -> Result<String> {
    let mut cmd = Command::new("nix");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let out = cmd
        .args(["eval", "--raw", "--accept-flake-config", installable])
        .output()
        .with_context(|| format!("spawning `nix eval {installable}`"))?;
    if !out.status.success() {
        bail!(
            "`nix eval {installable}` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)
        .with_context(|| format!("`nix eval {installable}` output was not UTF-8"))?
        .trim()
        .to_owned();
    if path.is_empty() {
        bail!("`nix eval {installable}` returned an empty path");
    }
    Ok(path)
}

/// Named derivation identities guarded by `nix probe-source`.
pub(crate) struct SourceProbeDrvPaths {
    pub(crate) static_docs: String,
    pub(crate) static_code: String,
    pub(crate) static_elisp: String,
    pub(crate) static_e2e_census: String,
    pub(crate) site: String,
    pub(crate) wasm_tests: String,
    pub(crate) xtask: String,
    pub(crate) devtool: String,
    pub(crate) tooling_dependencies: String,
    pub(crate) csr_debug_bundle: String,
}

/// Evaluate the derivation identities guarded by `nix probe-source`.
/// This deliberately performs only serial `nix eval` calls: the probe mutates
/// one isolated worktree between arms, so concurrent evaluation would obscure
/// which tracked perturbation produced a failed identity.
pub(crate) fn eval_source_probe_drvpaths(flake_dir: &Path) -> Result<SourceProbeDrvPaths> {
    Ok(SourceProbeDrvPaths {
        static_docs: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.static-docs.drvPath")?,
        static_code: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.static-code.drvPath")?,
        static_elisp: nix_eval_raw(
            Some(flake_dir),
            ".#checks.x86_64-linux.static-elisp.drvPath",
        )?,
        static_e2e_census: nix_eval_raw(
            Some(flake_dir),
            ".#checks.x86_64-linux.static-e2e-census.drvPath",
        )?,
        site: nix_eval_raw(Some(flake_dir), ".#site.drvPath")?,
        wasm_tests: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.wasm-tests.drvPath")?,
        xtask: nix_eval_raw(Some(flake_dir), ".#xtask.drvPath")?,
        devtool: nix_eval_raw(Some(flake_dir), ".#devtool.drvPath")?,
        tooling_dependencies: nix_eval_raw(
            Some(flake_dir),
            ".#devtool.toolingDependencies.drvPath",
        )?,
        csr_debug_bundle: nix_eval_raw(Some(flake_dir), ".#csrDebugBundle.drvPath")?,
    })
}

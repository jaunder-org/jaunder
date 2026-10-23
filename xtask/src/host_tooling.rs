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

const ADMIT_INPUTS: &str = r#"sourceAdmits:
let candidates = builtins.fromJSON (builtins.getEnv "JAUNDER_XTASK_UNTRACKED_INPUTS");
in map (candidate: candidate.path)
  (builtins.filter (candidate: sourceAdmits candidate.path candidate.kind) candidates)"#;

#[derive(Serialize)]
struct Candidate {
    path: String,
    kind: &'static str,
}

pub(crate) fn admit_untracked_inputs(root: &Path, source_admits: &str) -> Result<()> {
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
        .args(["eval", "--impure", "--json"])
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
            "Git flakes omit untracked declared inputs: {}. Track them with git add (or git add -N) before invoking xtask",
            omitted.join(", ")
        );
    }
    Ok(())
}

/// Select the caller's checkout, never the executable's Nix build directory.
/// Admission precedes hook installation, leases and command-owned result files.
pub fn prepare_checkout() -> Result<()> {
    let start = std::env::current_dir().context("resolving caller directory")?;
    let root = std::path::PathBuf::from(git::toplevel(&start)?);
    for marker in ["flake.nix", "xtask/Cargo.toml"] {
        if !std::fs::metadata(root.join(marker))
            .with_context(|| format!("reading Jaunder checkout marker {marker}"))?
            .is_file()
        {
            bail!("not a Jaunder checkout: {marker} is not a file");
        }
    }
    if !env!("JAUNDER_XTASK_NIX_OUTPUT").is_empty() {
        admit_untracked_inputs(&root, ".#xtask.sourceAdmits")?;
    }
    std::env::set_current_dir(root).context("selecting caller checkout")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_flake_admission_refuses_only_omitted_relevant_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let git = |args: &[&str]| {
            assert!(crate::git::at(root).args(args).status().unwrap().success());
        };
        git(&["init", "--quiet"]);
        // A dependency-free flake proves the real Git/Nix admission boundary
        // without realizing a toolchain or substituting a test verdict.
        std::fs::write(
            root.join("flake.nix"),
            r#"{
          outputs = { self }: { packages = builtins.listToAttrs (map (name: {
            inherit name;
            value.csrBundle = {
              sourceAdmits = path: kind: path == "new.rs";
              sourceText = builtins.readFile ./new.rs;
            };
          }) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); };
        }"#,
        )
        .unwrap();
        git(&["add", "flake.nix"]);
        std::fs::write(root.join("notes.md"), "harmless").unwrap();
        admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap();
        std::fs::write(root.join("new.rs"), "first source").unwrap();
        let error = admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap_err();
        assert!(error.to_string().contains("new.rs"));
        assert!(error.to_string().contains("git add -N"));
        let index = crate::git::at(root)
            .args(["ls-files", "new.rs"])
            .output()
            .unwrap();
        assert!(index.stdout.is_empty(), "admission must not stage input");
        let selected_text = || {
            let output = Command::new("nix")
                .args(["eval", "--raw", ".#csrBundle.sourceText"])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        git(&["add", "-N", "new.rs"]);
        admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap();
        assert_eq!(selected_text(), "first source");
        git(&["add", "new.rs"]);
        std::fs::write(root.join("new.rs"), "dirty source").unwrap();
        admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap();
        assert_eq!(selected_text(), "dirty source");
        std::fs::write(root.join("flake.nix"), "invalid Nix").unwrap();
        assert!(admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").is_err());
    }

    #[test]
    fn csr_admission_uses_the_real_producer_source_declarations() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let relevant = [
            "csr/src/new.rs",
            "web/src/new.rs",
            "client/src/new.rs",
            "common/src/new.rs",
            "macros/src/new.rs",
            "storage/src/new.rs",
            "tools/devtool/src/new.rs",
            "csr/index.html",
            "server/assets/jaunder.css",
            "host/system_theme_sources/new.css",
            "storage/migrations/new.sql",
            "tools/Cargo.lock",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "flake.nix",
            "flake.lock",
            "nix/new.nix",
        ];
        let candidates = relevant
            .iter()
            .chain(["notes.md", "server/src/unrelated.rs"].iter())
            .map(|path| Candidate {
                path: (*path).into(),
                kind: "regular",
            })
            .collect::<Vec<_>>();
        let output = Command::new("nix")
            .args([
                "eval",
                "--impure",
                "--json",
                ".#csrBundle.sourceAdmits",
                "--apply",
                ADMIT_INPUTS,
            ])
            .current_dir(root)
            .env(
                "JAUNDER_XTASK_UNTRACKED_INPUTS",
                serde_json::to_string(&candidates).unwrap(),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let admitted: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(admitted, relevant);
    }
}

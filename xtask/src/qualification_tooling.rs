//! Admission against Nix-selected executable outputs.
//!
//! Git admits the source that selects an output; the executable's identity is
//! its immutable store location. Build metadata and the actual running file must
//! agree, so a copied executable cannot claim the provenance of a store output.

use std::{fs, path::Path, process::Command};

use anyhow::{Context, Result, bail};

/// Verify a selected executable and its compiled Nix output before host effects.
/// No Git build revision is required when the same declared output is selected.
fn verify_selected_tool(
    selected: &Path,
    executing: &Path,
    compiled_output: Option<&Path>,
) -> Result<()> {
    let output = compiled_output.context("qualification requires Nix-built tooling provenance")?;
    if !output.starts_with("/nix/store") {
        bail!("qualification tooling provenance is not a Nix store output");
    }
    let selected = fs::canonicalize(selected).context("resolving selected Nix executable")?;
    let executing =
        fs::canonicalize(executing).context("resolving executing qualification tool")?;
    if selected != executing || executing.parent().and_then(Path::parent) != Some(output) {
        bail!("executing qualification tool does not match the selected Nix output");
    }
    Ok(())
}

/// Overrides may name the admitted helper (including a symlink to it), but
/// cannot substitute copied bytes or a different helper output.
fn verify_helper_override(selected: &Path, requested: Option<&Path>) -> Result<()> {
    let selected = fs::canonicalize(selected).context("resolving selected seed helper")?;
    if !selected.starts_with("/nix/store") {
        bail!("selected seed helper is not a Nix store executable");
    }
    if let Some(requested) = requested {
        let actual = fs::canonicalize(requested).context("resolving requested seed helper")?;
        if actual != selected {
            bail!("requested seed helper does not match the selected Nix output");
        }
    }
    Ok(())
}

/// Resolve the admitted source's existing Nix owners before allocating a lease.
pub(crate) fn admit_harness(
    source: &crate::qualification::SourcePin,
    gc_root: &Path,
) -> Result<crate::production_baseline_lifecycle::StoreArtifactIdentity> {
    let compiled = env!("JAUNDER_XTASK_NIX_OUTPUT");
    if compiled.is_empty() {
        bail!(
            "qualification requires Nix-built tooling provenance; invoke cargo xtask from the devShell"
        );
    }
    let installable = format!("{}#qualification-harness", source.locked_ref());
    let build: serde_json::Value = serde_json::from_slice(&nix_output(&[
        "build",
        "--out-link",
        gc_root.to_str().context("non-UTF-8 admission GC root")?,
        "--json",
        &installable,
    ])?)?;
    let entries = build
        .as_array()
        .context("Nix build did not return output records")?;
    if entries.len() != 1 {
        bail!("qualification harness must select exactly one derivation");
    }
    let entry = &entries[0];
    let derivation = entry["drvPath"]
        .as_str()
        .context("Nix harness omitted drvPath")?
        .to_owned();
    let output_path = entry["outputs"]["out"]
        .as_str()
        .context("Nix harness omitted its out output")?
        .to_owned();
    verify_selected_tool(
        &Path::new(&output_path).join("bin/xtask"),
        &std::env::current_exe().context("resolving executing qualification tool")?,
        Some(Path::new(compiled)),
    )?;
    let requested_helper =
        std::env::var_os("JAUNDER_TEST_SUPPORT_BIN").map(std::path::PathBuf::from);
    verify_helper_override(
        &Path::new(&output_path).join("bin/test-support"),
        requested_helper.as_deref(),
    )?;
    let info: serde_json::Value =
        serde_json::from_slice(&nix_output(&["path-info", "--json", &output_path])?)?;
    let entries = info
        .as_object()
        .context("Nix path-info must be keyed by store output")?;
    if entries.len() != 1 || !entries.contains_key(&output_path) {
        bail!("Nix path-info did not identify the selected harness output");
    }
    let nar_hash = entries[&output_path]["narHash"]
        .as_str()
        .context("Nix harness path-info omitted narHash")?
        .to_owned();
    Ok(
        crate::production_baseline_lifecycle::StoreArtifactIdentity {
            derivation,
            output_path,
            nar_hash,
        },
    )
}

fn nix_output(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("nix")
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .output()
        .context("resolving Nix qualification inputs")?;
    if !output.status.success() {
        bail!(
            "Nix qualification input resolution failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_executable() -> std::path::PathBuf {
        let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join("bash"))
            .find(|path| path.is_file())
            .unwrap();
        let executable = fs::canonicalize(executable).unwrap();
        assert!(executable.starts_with("/nix/store"));
        executable
    }

    #[test]
    fn helper_override_must_resolve_to_the_selected_store_executable() {
        let selected = store_executable();
        verify_helper_override(&selected, None).unwrap();
        verify_helper_override(&selected, Some(&selected)).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let copied = temp.path().join("copied-helper");
        fs::copy(&selected, &copied).unwrap();
        let error = verify_helper_override(&selected, Some(&copied)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match the selected Nix output")
        );
    }

    #[test]
    fn missing_nix_provenance_is_refused_before_executable_access() {
        let error = verify_selected_tool(
            Path::new("/absent/selected"),
            Path::new("/absent/executing"),
            None,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires Nix-built tooling provenance")
        );
    }

    #[test]
    fn selected_tool_is_reusable_without_a_git_build_revision() {
        let executable = store_executable();
        let output = executable.parent().unwrap().parent().unwrap();
        assert!(verify_selected_tool(&executable, &executable, Some(output)).is_ok());
    }

    #[test]
    fn copied_executable_cannot_claim_its_original_store_output() {
        let executable = store_executable();
        let output = executable.parent().unwrap().parent().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let copied = temp.path().join("copied-tool");
        fs::copy(&executable, &copied).unwrap();
        let error = verify_selected_tool(&executable, &copied, Some(output)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match the selected Nix output")
        );
    }

    #[test]
    fn mixed_output_metadata_is_refused_even_when_executable_paths_match() {
        let executable = store_executable();
        let error = verify_selected_tool(
            &executable,
            &executable,
            Some(Path::new("/nix/store/wrong-output")),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match the selected Nix output")
        );
    }
}

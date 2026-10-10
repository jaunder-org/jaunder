//! Admission against Nix-selected executable outputs.
//!
//! Git admits the source that selects an output; the executable's identity is
//! its immutable store location. Build metadata and the actual running file must
//! agree, so a copied executable cannot claim the provenance of a store output.

use std::{fs, path::Path, process::Command};

use anyhow::{Context, Result, bail};

/// Verify a selected executable and its compiled Nix output before host effects.
/// No Git build revision is required when the same declared output is selected.
pub fn verify_selected_tool(
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
pub fn verify_helper_override(selected: &Path, requested: Option<&Path>) -> Result<()> {
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

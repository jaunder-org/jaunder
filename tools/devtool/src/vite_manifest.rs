//! Narrow reads of the standard Vite build manifest used by CSR diagnostics.
//!
//! This deliberately exposes output paths, not a project-owned asset role or
//! representation schema.

use std::{
    fs,
    path::{Component, Path},
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

/// Resolve the wasm-bindgen input key to its emitted Vite output path.
///
/// # Errors
///
/// Returns an error if the manifest entry or its output is absent, malformed,
/// or escapes the bundle root.
pub(crate) fn wasm_output(root: &Path) -> Result<String> {
    let manifest_path = root.join(".vite/manifest.json");
    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("reading Vite manifest {}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing Vite manifest {}", manifest_path.display()))?;
    let output = manifest
        .get("csr_bg.wasm")
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("file"))
        .and_then(Value::as_str)
        .context("Vite manifest has no output file for csr_bg.wasm")?;
    let path = Path::new(output);
    if output.is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("unsafe Vite WASM output path: {output:?}");
    }
    ensure!(
        path.extension()
            .is_some_and(|extension| extension == "wasm"),
        "Vite csr_bg.wasm output must have a .wasm extension"
    );
    ensure!(
        root.join(path).is_file(),
        "Vite WASM output is missing: {output}"
    );
    Ok(output.to_owned())
}

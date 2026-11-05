//! Minimal reads of Vite's build-only manifest for CSR consumers.
//!
//! This adapter exposes only the standard output paths consumers need. It does
//! not define or serialize a Jaunder manifest schema.

use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CsrViteOutputs {
    /// The JavaScript file emitted for Vite's `index.html` input entry.
    pub entry_js: String,
    /// The file emitted for wasm-bindgen's `csr_bg.wasm` input asset.
    pub wasm: String,
    /// Emitted file paths listed by Vite's ordinary output manifest.
    pub files: Vec<String>,
}

/// Read the pinned Vite manifest, resolve the stable source entry names and
/// check that every manifest-owned output exists below `root`.
///
/// # Errors
///
/// Returns an error when Vite's expected HTML/WASM inputs or output files are
/// missing, malformed, unsafe, or absent from the output tree.
pub fn csr_outputs(root: &Path) -> Result<CsrViteOutputs> {
    let manifest_path = root.join(".vite/manifest.json");
    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("reading Vite manifest {}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing Vite manifest {}", manifest_path.display()))?;
    let entries = manifest
        .as_object()
        .context("Vite manifest root must be an object")?;
    let file_for = |key: &str| -> Result<String> {
        let output = entries
            .get(key)
            .and_then(Value::as_object)
            .and_then(|entry| entry.get("file"))
            .and_then(Value::as_str)
            .with_context(|| format!("Vite manifest has no output file for {key:?}"))?;
        validate_output_path(output)?;
        ensure!(
            root.join(output).is_file(),
            "Vite output file is missing: {output}"
        );
        Ok(output.to_owned())
    };

    let entry_js = file_for("index.html")?;
    let wasm = file_for("csr_bg.wasm")?;
    ensure!(
        Path::new(&wasm)
            .extension()
            .is_some_and(|ext| ext == "wasm"),
        "Vite csr_bg.wasm mapping must resolve to a WASM output"
    );

    let mut files = BTreeSet::new();
    for (key, value) in entries {
        let entry = value
            .as_object()
            .with_context(|| format!("Vite manifest entry {key:?} must be an object"))?;
        if let Some(file) = entry.get("file").and_then(Value::as_str) {
            validate_output_path(file)?;
            ensure!(
                root.join(file).is_file(),
                "Vite output file is missing: {file}"
            );
            files.insert(file.to_owned());
        }
        for field in ["css", "assets"] {
            if let Some(paths) = entry.get(field) {
                let paths = paths.as_array().with_context(|| {
                    format!("Vite manifest entry {key:?} field {field:?} must be an array")
                })?;
                for value in paths {
                    let file = value.as_str().with_context(|| {
                        format!("Vite manifest entry {key:?} field {field:?} must contain paths")
                    })?;
                    validate_output_path(file)?;
                    ensure!(
                        root.join(file).is_file(),
                        "Vite output file is missing: {file}"
                    );
                    files.insert(file.to_owned());
                }
            }
        }
    }

    Ok(CsrViteOutputs {
        entry_js,
        wasm,
        files: files.into_iter().collect(),
    })
}

fn validate_output_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe Vite output path: {value:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn output(root: &Path, wasm_key: &str) {
        fs::create_dir_all(root.join(".vite")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("assets/entry.js"), b"entry").unwrap();
        fs::write(root.join("assets/client.wasm"), b"wasm").unwrap();
        fs::write(
            root.join(".vite/manifest.json"),
            format!(
                r#"{{
                  "index.html": {{"file":"assets/entry.js","src":"index.html","isEntry":true,"assets":["assets/client.wasm"]}},
                  {wasm_key:?}: {{"file":"assets/client.wasm","src":"{wasm_key}"}}
                }}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn selects_standard_html_entry_and_logical_wasm_source() {
        let root = tempdir().unwrap();
        output(root.path(), "csr_bg.wasm");
        assert_eq!(
            csr_outputs(root.path()).unwrap(),
            CsrViteOutputs {
                entry_js: "assets/entry.js".into(),
                wasm: "assets/client.wasm".into(),
                files: vec!["assets/client.wasm".into(), "assets/entry.js".into()],
            }
        );
    }

    #[test]
    fn rejects_missing_or_unsafe_manifest_outputs() {
        let root = tempdir().unwrap();
        output(root.path(), "csr_bg.wasm");
        fs::write(
            root.path().join(".vite/manifest.json"),
            r#"{"index.html":{"file":"../outside.js"},"csr_bg.wasm":{"file":"assets/client.wasm"}}"#,
        )
        .unwrap();
        assert!(csr_outputs(root.path()).is_err());
    }
}

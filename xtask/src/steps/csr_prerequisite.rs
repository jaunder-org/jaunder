//! Complete CSR input validation for startup-capable host tests.
//!
//! Integrity checks do not establish source provenance. Explicit inputs must be
//! intentionally source-matched and kept stable by their owner through compilation.

use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};

/// Resolve one absolute input and compose the existing build-contract validators.
pub(crate) fn validate(root: &Path) -> anyhow::Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving CSR bundle root {}", root.display()))?;
    let manifest_check = crate::invocation_trace::active()
        .then(|| crate::invocation_trace::phase("xtask.csr-manifest-check"));
    let manifest_result = host::vite_manifest::csr_outputs(&root)
        .with_context(|| format!("invalid Vite manifest or output under {}", root.display()));
    if let Some(phase) = manifest_check {
        phase.finish(manifest_result.is_ok());
    }
    manifest_result?;
    let inventory_check = crate::invocation_trace::active()
        .then(|| crate::invocation_trace::phase("xtask.csr-inventory-check"));
    let inventory_result = host::system_theme::load_system_artifact_inventory(
        &host::system_theme::DirectorySystemArtifactSource::new(&root.join("system-artifacts")),
    )
    .context("invalid system artifacts");
    if let Some(phase) = inventory_check {
        phase.finish(inventory_result.is_ok());
    }
    inventory_result?;
    ensure!(
        root.join("index.html").is_file(),
        "CSR bundle has no rendered index.html"
    );
    Ok(root)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use super::*;
    // Sanitized Vite contract fixture; it is not runnable wasm or a startup proof.
    pub(crate) fn write_bundle(root: &Path) -> host::vite_manifest::CsrViteOutputs {
        let inventory = host::system_theme::qualification::compile(
            host::system_theme::qualification::Fixture::A,
        )
        .unwrap();
        host::system_theme::stage_system_artifact_inventory(
            &inventory,
            &root.join("system-artifacts"),
        )
        .unwrap();
        let entry = "assets/index-a1b2c3d4.js";
        let wasm = "assets/csr_bg-e5f6a7b8.wasm";
        fs::create_dir_all(root.join(".vite")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join(entry), b"entry").unwrap();
        fs::write(root.join(wasm), b"wasm").unwrap();
        fs::write(
            root.join(".vite/manifest.json"),
            format!(
                r#"{{"index.html":{{"file":"{entry}","src":"index.html","isEntry":true,"assets":["{wasm}"]}},"csr_bg.wasm":{{"file":"{wasm}","src":"csr_bg.wasm"}}}}"#
            ),
        )
        .unwrap();
        let application = inventory.application().content_digest().content_url();
        fs::write(root.join("index.html"), format!(
            r#"<script>const __jaunderWasmUrl = "/{wasm}"; window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);</script>
<link rel="stylesheet" href="{application}" data-jaunder-application-stylesheet />
<script type="module" crossorigin src="/{entry}"></script>"#,
        )).unwrap();
        host::vite_manifest::csr_outputs(root).unwrap()
    }

    #[test]
    fn traced_validation_phases_are_nested_and_stop_after_failure() {
        let temp = tempfile::tempdir().unwrap();
        write_bundle(temp.path());
        let capture = crate::invocation_trace::Capture::new();
        capture.with_default(|| {
            let command = crate::invocation_trace::phase("xtask.command");
            validate(temp.path()).unwrap();
            crate::invocation_trace::command_outcome(true, 0, None);
            command.finish(true);
        });
        let trace = capture.finish().unwrap();
        let events: Vec<serde_json::Value> = serde_json::from_slice(&trace.bytes).unwrap();
        let names = ["xtask.csr-manifest-check", "xtask.csr-inventory-check"];
        let begins: Vec<_> = events
            .iter()
            .filter(|event| {
                names.contains(&event["name"].as_str().unwrap_or("")) && event["ph"] == "B"
            })
            .map(|event| event["name"].as_str().unwrap())
            .collect();
        assert_eq!(begins, names);
        let stack: Vec<_> = events
            .iter()
            .filter(|event| names.contains(&event["name"].as_str().unwrap_or("")))
            .filter_map(|event| match event["ph"].as_str() {
                Some("B") => Some(event["name"].as_str().unwrap()),
                Some("E") => Some("end"),
                _ => None,
            })
            .collect();
        assert_eq!(stack, [names[0], "end", names[1], "end"]);

        fs::write(temp.path().join(".vite/manifest.json"), "corrupt").unwrap();
        let capture = crate::invocation_trace::Capture::new();
        capture.with_default(|| {
            let command = crate::invocation_trace::phase("xtask.command");
            assert!(validate(temp.path()).is_err());
            crate::invocation_trace::command_outcome(false, 1, None);
            command.finish(false);
        });
        let trace = capture.finish().unwrap();
        let events: Vec<serde_json::Value> = serde_json::from_slice(&trace.bytes).unwrap();
        assert!(events.iter().any(|event| {
            event["name"] == "xtask.phase.outcome"
                && event["args"]["phase"] == names[0]
                && event["args"]["outcome"] == "failure"
        }));
        assert!(!events.iter().any(|event| event["name"] == names[1]));
    }

    #[test]
    fn complete_input_resolves_one_absolute_root() {
        let temp = tempfile::tempdir().unwrap();
        write_bundle(temp.path());
        assert_eq!(
            validate(&temp.path().join(".")).unwrap(),
            temp.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn rejects_missing_and_corrupt_manifest_representation_and_inventory() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            validate(&temp.path().join("missing"))
                .unwrap_err()
                .to_string()
                .contains("resolving CSR bundle root")
        );
        assert!(
            format!("{:#}", validate(temp.path()).unwrap_err()).contains("reading Vite manifest")
        );
        for corruption in ["manifest", "output", "inventory", "index"] {
            let temp = tempfile::tempdir().unwrap();
            let outputs = write_bundle(temp.path());
            let (path, expected) = match corruption {
                "manifest" => (
                    temp.path().join(".vite/manifest.json"),
                    "parsing Vite manifest",
                ),
                "output" => (
                    temp.path().join(&outputs.wasm),
                    "Vite output file is missing",
                ),
                "inventory" => (
                    temp.path().join("system-artifacts/inventory.txt"),
                    "invalid system artifacts",
                ),
                _ => (temp.path().join("index.html"), "no rendered index.html"),
            };
            if corruption == "output" || corruption == "index" {
                fs::remove_file(path).unwrap();
            } else {
                fs::write(path, b"corrupt").unwrap();
            }
            let error = validate(temp.path()).unwrap_err();
            assert!(
                format!("{error:#}").contains(expected),
                "{corruption}: {error:#}"
            );
        }
    }
}

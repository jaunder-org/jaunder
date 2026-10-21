//! Complete CSR input validation for startup-capable host tests.
//!
//! Integrity checks do not establish source provenance. Explicit inputs must be
//! intentionally source-matched and kept stable by their owner through compilation.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use csr_bundle::{Manifest, Role, validate_rendered_shell};

/// Resolve one absolute input and compose the existing build-contract validators.
pub(crate) fn validate(root: &Path) -> anyhow::Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving CSR bundle root {}", root.display()))?;
    let manifest_path = root.join("manifest.json");
    let bytes =
        fs::read(&manifest_path).with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = Manifest::from_json(&bytes)
        .with_context(|| format!("invalid {}", manifest_path.display()))?;
    manifest
        .verify_bundle(&root)
        .with_context(|| format!("invalid bundle {}", root.display()))?;
    let inventory = host::system_theme::load_system_artifact_inventory(
        &host::system_theme::DirectorySystemArtifactSource::new(&root.join("system-artifacts")),
    )
    .context("invalid system artifacts")?;
    let shell =
        fs::read_to_string(root.join("index.html")).context("reading rendered index.html")?;
    let glue = manifest
        .role(Role::Glue)
        .context("verified glue role missing")?;
    let wasm = manifest
        .role(Role::Wasm)
        .context("verified WASM role missing")?;
    validate_rendered_shell(
        &shell,
        &format!("/{}", glue.path),
        &format!("/{}", wasm.path),
        inventory
            .application()
            .content_digest()
            .content_url()
            .as_ref(),
    )?;
    Ok(root)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use csr_bundle::{Asset, Representation};
    use std::collections::BTreeMap;

    // Sanitized contract fixture; it is not runnable wasm or a startup proof.
    pub(crate) fn write_bundle(root: &Path) -> Manifest {
        let inventory = host::system_theme::qualification::compile(
            host::system_theme::qualification::Fixture::A,
        )
        .unwrap();
        host::system_theme::stage_system_artifact_inventory(
            &inventory,
            &root.join("system-artifacts"),
        )
        .unwrap();
        let mut assets = Vec::new();
        for (role, extension, bytes) in [
            (Role::Glue, "js", b"glue".as_slice()),
            (Role::Wasm, "wasm", b"wasm".as_slice()),
        ] {
            let digest = csr_bundle::digest(bytes);
            let path = format!("pkg/{digest}.{extension}");
            let mut representations = BTreeMap::new();
            for (encoding, suffix, bytes) in [
                ("identity", "", bytes),
                ("gzip", ".gz", b"gzip".as_slice()),
                ("br", ".br", b"brotli".as_slice()),
            ] {
                let representation_path = format!("{path}{suffix}");
                fs::create_dir_all(root.join("pkg")).unwrap();
                fs::write(root.join(&representation_path), bytes).unwrap();
                representations.insert(
                    encoding.into(),
                    Representation {
                        path: representation_path,
                        sha256: csr_bundle::digest(bytes),
                    },
                );
            }
            assets.push(Asset {
                role: Some(role),
                path,
                sha256: digest,
                representations,
            });
        }
        let manifest = Manifest {
            version: csr_bundle::VERSION,
            assets,
        };
        fs::write(root.join("manifest.json"), manifest.to_json().unwrap()).unwrap();
        let glue = &manifest.role(Role::Glue).unwrap().path;
        let wasm = &manifest.role(Role::Wasm).unwrap().path;
        let application = inventory.application().content_digest().content_url();
        fs::write(root.join("index.html"), format!(
            r#"const __jaunderWasmUrl = "/{wasm}"; window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);
<link rel="stylesheet" href="{application}" data-jaunder-application-stylesheet />
import {{initMeasured}} from "/{glue}"; performance.mark("before_init");
initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl)"#,
        )).unwrap();
        manifest
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
    fn rejects_missing_and_corrupt_manifest_representation_shell_and_inventory() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            validate(&temp.path().join("missing"))
                .unwrap_err()
                .to_string()
                .contains("resolving CSR bundle root")
        );
        assert!(
            validate(temp.path())
                .unwrap_err()
                .to_string()
                .contains("reading")
        );
        for corruption in ["manifest", "representation", "shell", "inventory"] {
            let temp = tempfile::tempdir().unwrap();
            let manifest = write_bundle(temp.path());
            let (path, expected) = match corruption {
                "manifest" => (temp.path().join("manifest.json"), "invalid manifest JSON"),
                "representation" => (
                    temp.path().join(&manifest.assets[0].path),
                    "digest mismatch",
                ),
                "shell" => (temp.path().join("index.html"), "staged shell"),
                _ => (
                    temp.path().join("system-artifacts/inventory.txt"),
                    "invalid system artifacts",
                ),
            };
            fs::write(path, b"corrupt").unwrap();
            let error = validate(temp.path()).unwrap_err();
            assert!(
                format!("{error:#}").contains(expected),
                "{corruption}: {error:#}"
            );
        }
    }
}

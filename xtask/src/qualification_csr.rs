//! Canonical CSR-bundle snapshotting for qualification package provenance.

use anyhow::{Context, Result, bail};
use host::vite_manifest::csr_outputs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CsrResourceIdentity {
    pub path: String,
    pub sha256: String,
    pub representations: BTreeMap<String, (String, String)>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CsrPayloadSnapshot {
    pub manifest_sha256: String,
    pub normalized_index_sha256: String,
    pub assets: Vec<CsrResourceIdentity>,
    pub application_urls_in_index: Vec<String>,
}

pub(crate) fn snapshot_csr_bundle(root: &Path) -> Result<CsrPayloadSnapshot> {
    let manifest_path = root.join(".vite/manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .with_context(|| format!("reading Vite manifest {}", manifest_path.display()))?;
    let outputs = csr_outputs(root)?;
    let index = fs::read_to_string(root.join("index.html")).context("reading CSR bundle index")?;
    let (application_url, normalized_index_sha256) = snapshot_index(&index)?;
    let application_urls_in_index = vec![application_url];
    let mut assets = Vec::with_capacity(outputs.files.len());
    for relative in outputs.files {
        let path = root.join(&relative);
        let bytes =
            fs::read(&path).with_context(|| format!("reading Vite output {}", path.display()))?;
        let mut representations = BTreeMap::new();
        for (encoding, suffix) in [("gzip", ".gz"), ("br", ".br")] {
            let sidecar = format!("{relative}{suffix}");
            let sidecar_path = root.join(&sidecar);
            if sidecar_path.is_file() {
                let sidecar_bytes = fs::read(&sidecar_path)
                    .with_context(|| format!("reading Vite sidecar {}", sidecar_path.display()))?;
                representations.insert(
                    encoding.to_owned(),
                    (
                        sidecar,
                        crate::digest::lowercase_hex(Sha256::digest(sidecar_bytes)),
                    ),
                );
            } else if matches!(
                Path::new(&relative)
                    .extension()
                    .and_then(|ext| ext.to_str()),
                Some("js" | "wasm")
            ) {
                bail!("Vite {encoding} sidecar is missing for {}", path.display());
            }
        }
        assets.push(CsrResourceIdentity {
            path: relative,
            sha256: crate::digest::lowercase_hex(Sha256::digest(bytes)),
            representations,
        });
    }
    Ok(CsrPayloadSnapshot {
        manifest_sha256: crate::digest::lowercase_hex(Sha256::digest(&manifest_bytes)),
        normalized_index_sha256,
        assets,
        application_urls_in_index,
    })
}
// The trusted producer emits a single ordinary stylesheet link. Validate that
// closed shell shape and normalize only its address, never arbitrary HTML bytes.
fn snapshot_index(index: &str) -> Result<(String, String)> {
    let links = regex::Regex::new(r#"<link\b[^>]*\srel="stylesheet"[^>]*>"#)?;
    let mut links = links.find_iter(index);
    let link = links
        .next()
        .context("CSR shell omits its application stylesheet")?;
    if links.next().is_some() {
        bail!("CSR shell must contain exactly one application stylesheet");
    }
    let href = regex::Regex::new(r#"\shref="(/theme/[0-9a-f]{64})""#)?;
    let captures = href
        .captures(link.as_str())
        .context("CSR application URL is not canonical")?;
    let address = captures
        .get(1)
        .context("CSR application URL capture missing")?;
    let start = link.start() + address.start();
    let end = link.start() + address.end();
    let normalized = format!(
        "{}{{{{APPLICATION_STYLESHEET_URL}}}}{}",
        &index[..start],
        &index[end..],
    );
    Ok((
        address.as_str().to_owned(),
        crate::digest::lowercase_hex(Sha256::digest(normalized.as_bytes())),
    ))
}

pub(crate) fn compare_csr_payloads(
    a: &CsrPayloadSnapshot,
    b: &CsrPayloadSnapshot,
    application_url_changed: bool,
) -> Result<()> {
    if a.assets != b.assets || a.manifest_sha256 != b.manifest_sha256 {
        bail!("qualification CSR runtime assets or manifest differ")
    }
    if a.normalized_index_sha256 != b.normalized_index_sha256 {
        bail!("qualification CSR shell changed beyond its application address")
    }
    if application_url_changed {
        if a.application_urls_in_index == b.application_urls_in_index {
            bail!("B-app CSR index did not change its application stylesheet URL")
        }
    } else if a.application_urls_in_index != b.application_urls_in_index {
        bail!("CSR index application stylesheet URLs changed unexpectedly")
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fixture_bundle(root: &Path) {
        let entry = "assets/index-a1b2c3d4.js";
        let wasm = "assets/csr_bg-e5f6a7b8.wasm";
        fs::create_dir_all(root.join(".vite")).unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        for (path, bytes) in [(entry, b"export {}".as_slice()), (wasm, b"\0asm\x01\0\0\0")] {
            fs::write(root.join(path), bytes).unwrap();
            fs::write(root.join(format!("{path}.gz")), b"gzip output").unwrap();
            fs::write(root.join(format!("{path}.br")), b"brotli output").unwrap();
        }
        fs::write(
            root.join(".vite/manifest.json"),
            format!(
                r#"{{"index.html":{{"file":"{entry}","src":"index.html","isEntry":true,"assets":["{wasm}"]}},"csr_bg.wasm":{{"file":"{wasm}","src":"csr_bg.wasm"}}}}"#
            ),
        )
        .unwrap();
        let stylesheet = format!("/theme/{}", "a".repeat(64));
        fs::write(
            root.join("index.html"),
            format!(r#"<script type="module" src="/{entry}"></script><link rel="stylesheet" href="{stylesheet}" />"#),
        )
        .unwrap();
    }
    #[test]
    fn producer_shell_normalizes_only_the_application_address() {
        let a = format!(
            r#"<head><link rel="stylesheet" href="/theme/{}" /></head>"#,
            "a".repeat(64)
        );
        let b = a.replace(&"a".repeat(64), &"b".repeat(64));
        let (url, normalized) = snapshot_index(&a).unwrap();
        assert_eq!(url, format!("/theme/{}", "a".repeat(64)));
        assert_eq!(snapshot_index(&b).unwrap().1, normalized);
        assert_ne!(
            snapshot_index(&a.replace("<head>", "<head>changed"))
                .unwrap()
                .1,
            normalized
        );
        assert!(snapshot_index(&format!("{a}{a}")).is_err());
        assert!(snapshot_index(&a.replace(&"a".repeat(64), "invalid")).is_err());
    }

    #[test]
    fn native_manifest_snapshot_captures_outputs_and_sidecars() {
        let root = tempdir().unwrap();
        fixture_bundle(root.path());
        let snapshot = snapshot_csr_bundle(root.path()).unwrap();
        assert_eq!(snapshot.assets.len(), 2);
        let wasm = snapshot
            .assets
            .iter()
            .find(|asset| asset.path.ends_with(".wasm"))
            .unwrap();
        let (gzip_path, gzip_sha256) = wasm.representations.get("gzip").unwrap();
        assert!(gzip_path.ends_with(".wasm.gz"));
        assert_eq!(gzip_sha256.len(), 64);
        let (brotli_path, brotli_sha256) = wasm.representations.get("br").unwrap();
        assert!(brotli_path.ends_with(".wasm.br"));
        assert_eq!(brotli_sha256.len(), 64);
        assert_eq!(wasm.sha256.len(), 64);
        assert_eq!(snapshot.manifest_sha256.len(), 64);
        let missing = tempdir().unwrap();
        fs::create_dir_all(missing.path().join(".vite")).unwrap();
        fs::write(missing.path().join(".vite/manifest.json"), "not-json").unwrap();
        assert!(snapshot_csr_bundle(missing.path()).is_err());
        let missing_sidecar = tempdir().unwrap();
        fixture_bundle(missing_sidecar.path());
        fs::remove_file(
            missing_sidecar
                .path()
                .join("assets/csr_bg-e5f6a7b8.wasm.br"),
        )
        .unwrap();
        assert!(
            format!(
                "{:#}",
                snapshot_csr_bundle(missing_sidecar.path()).unwrap_err()
            )
            .contains("Vite br sidecar is missing")
        );
        let changed = tempdir().unwrap();
        fixture_bundle(changed.path());
        fs::write(
            changed.path().join("assets/csr_bg-e5f6a7b8.wasm.br"),
            b"changed br",
        )
        .unwrap();
        let changed = snapshot_csr_bundle(changed.path()).unwrap();
        assert!(compare_csr_payloads(&snapshot, &changed, false).is_err());
        let a = CsrPayloadSnapshot {
            manifest_sha256: "a".repeat(64),
            normalized_index_sha256: "c".repeat(64),
            assets: vec![],
            application_urls_in_index: vec!["/theme/a".into()],
        };
        let mut b = a.clone();
        b.assets.push(CsrResourceIdentity {
            path: "pkg/x".into(),
            sha256: "b".repeat(64),
            representations: BTreeMap::new(),
        });
        assert!(compare_csr_payloads(&a, &b, false).is_err());
    }
}

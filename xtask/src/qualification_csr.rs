//! Canonical CSR-bundle snapshotting for qualification package provenance.

use anyhow::{Context, Result, bail};
use csr_bundle::Manifest;
use serde::{Deserialize, Serialize};
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
    let manifest_bytes =
        fs::read(root.join("manifest.json")).context("reading CSR bundle manifest")?;
    let manifest = Manifest::from_json(&manifest_bytes)?;
    manifest.verify_bundle(root)?;
    let index = fs::read_to_string(root.join("index.html")).context("reading CSR bundle index")?;
    let (application_url, normalized_index_sha256) = snapshot_index(&index)?;
    let application_urls_in_index = vec![application_url];
    let assets = manifest
        .assets
        .into_iter()
        .map(|asset| CsrResourceIdentity {
            path: asset.path,
            sha256: asset.sha256,
            representations: asset
                .representations
                .into_iter()
                .map(|(encoding, representation)| {
                    (encoding, (representation.path, representation.sha256))
                })
                .collect(),
        })
        .collect();
    Ok(CsrPayloadSnapshot {
        manifest_sha256: csr_bundle::digest(&manifest_bytes),
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
        csr_bundle::digest(normalized.as_bytes()),
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
    fn canonical_manifest_parser_rejects_invalid_manifest() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("manifest.json"), b"{}").unwrap();
        assert!(snapshot_csr_bundle(root.path()).is_err());
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

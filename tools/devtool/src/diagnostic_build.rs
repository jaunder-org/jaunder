//! Typed artifact operations for the diagnostic CSR WebAssembly build.
//!
//! Nix owns process orchestration; this module owns every structured file and
//! filesystem selection that defines the diagnostic build's observable output.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PATH_RELATIONSHIP: &str = "The retained module is linked from LLVM IR compiled below compiled_source_prefix; the content-addressed wasm selected by pkg/manifest.json is its wasm-bindgen/wasm-opt derivative.";
const SERVED_RELATIONSHIP: &str = "input to wasm-bindgen and wasm-opt that produced the content-addressed module selected by pkg/manifest.json";

#[derive(Serialize)]
struct SourceIdentity<'a> {
    version: u8,
    source_identity: SourceIdentityValue<'a>,
    compilation_directory: String,
    path_equivalence: PathEquivalence,
}

#[derive(Serialize)]
struct SourceIdentityValue<'a> {
    kind: &'static str,
    value: &'a str,
}

#[derive(Serialize)]
struct PathEquivalence {
    compiled_source_prefix: String,
    retained_source_mappable_module: &'static str,
    relationship: &'static str,
}

#[derive(Serialize, Deserialize)]
struct IrManifest {
    version: u8,
    root_ir_selection: Vec<String>,
    root_rlink_selection: Vec<String>,
    instrumented_root_ir: String,
    rustc_link_metadata: String,
    cargo_artifacts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    linked_wasm_selection: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linked_wasm: Option<String>,
}

#[derive(Deserialize)]
struct CoverageMetadata {
    result: String,
    input: CoverageInput,
}

#[derive(Deserialize)]
struct CoverageInput {
    wasm_bindgen_metadata: bool,
}

#[derive(Deserialize)]
struct BundleManifest {
    assets: Vec<BundleAsset>,
}

#[derive(Deserialize)]
struct BundleAsset {
    path: String,
    role: Option<String>,
}

#[derive(Serialize)]
struct ServedModule {
    path: String,
    sha256: String,
}

#[derive(Serialize)]
struct SourceMappableModule {
    path: &'static str,
    sha256: String,
    relationship_to_served_module: &'static str,
}

#[derive(Serialize)]
struct InstrumentedStatus {
    version: u8,
    outcome: &'static str,
    pipeline_exit: i32,
    module: Option<&'static str>,
    bundle: Option<String>,
    served_module: Option<ServedModule>,
    source_mappable_module: Option<SourceMappableModule>,
    source_identity: &'static str,
    coverage_metadata: Option<&'static str>,
    toolchain_identity: Option<&'static str>,
    ir_manifest: Option<&'static str>,
    diagnostic_log: &'static str,
}

#[derive(Serialize)]
struct BaselineStatus {
    version: u8,
    outcome: &'static str,
    served_module: ServedModule,
    unavoidable_deviations: [&'static str; 2],
}

pub fn prepare_source(
    source: &Path,
    minicov: &Path,
    runtime: &Path,
    source_identity: &Path,
    nix_source: &Path,
) -> Result<()> {
    let root_manifest = source.join("Cargo.toml");
    let root = fs::read_to_string(&root_manifest).context("reading copied workspace manifest")?;
    fs::write(
        &root_manifest,
        root.replacen(
            "[patch.crates-io]\n",
            &format!(
                "[patch.crates-io]\nminicov = {{ path = \"{}\" }}\n",
                minicov.display()
            ),
            1,
        ),
    )?;

    let client_manifest = source.join("client/Cargo.toml");
    let client = fs::read_to_string(&client_manifest).context("reading copied client manifest")?;
    fs::write(
        client_manifest,
        format!(
            "{}\n[target.'cfg(target_arch = \"wasm32\")'.dependencies]\ndiagnostic-coverage-runtime = {{ path = \"{}\", optional = true }}\n",
            client.replacen(
                "diagnostic-coverage = []",
                "diagnostic-coverage = [\"dep:diagnostic-coverage-runtime\"]",
                1,
            ),
            runtime.display(),
        ),
    )?;

    let nix_source = nix_source.display().to_string();
    let compilation_directory = source.canonicalize()?.display().to_string();
    write_json(
        source_identity,
        &SourceIdentity {
            version: 1,
            source_identity: SourceIdentityValue {
                kind: "nix-store-source",
                value: &nix_source,
            },
            compilation_directory: compilation_directory.clone(),
            path_equivalence: PathEquivalence {
                compiled_source_prefix: compilation_directory,
                retained_source_mappable_module: "instrumented/csr.wasm",
                relationship: PATH_RELATIONSHIP,
            },
        },
    )
}

pub fn validate_llvm(rustc_version: &Path, clang_version: &Path) -> Result<()> {
    let rustc = fs::read_to_string(rustc_version)?;
    let clang = fs::read_to_string(clang_version)?;
    let rust_major = rustc
        .lines()
        .find_map(|line| line.strip_prefix("LLVM version: "))
        .and_then(|version| version.split('.').next());
    let clang_major = clang
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(3)
        .find_map(|parts| {
            (parts[0] == "clang" && parts[1] == "version")
                .then(|| parts[2].split('.').next())
                .flatten()
        });
    let (Some(rust_major), Some(clang_major)) = (rust_major, clang_major) else {
        bail!("could not establish Rust and Clang LLVM major versions");
    };
    if rust_major != "22" || clang_major != "22" {
        bail!("coverage pipeline requires LLVM 22; rustc={rust_major} clang={clang_major}");
    }
    Ok(())
}

pub fn discover_root(
    target_release: &Path,
    root_ir: &Path,
    root_rlink: &Path,
    manifest: &Path,
) -> Result<()> {
    let ir = select_one(target_release, "ll")?;
    let rlink = select_one(target_release, "rlink")?;
    fs::write(root_ir, format!("{}\n", ir.display()))?;
    fs::write(root_rlink, format!("{}\n", rlink.display()))?;
    write_json(
        manifest,
        &IrManifest {
            version: 4,
            root_ir_selection: vec!["release/csr*.ll".into(), "release/deps/csr*.ll".into()],
            root_rlink_selection: vec![
                "release/csr*.rlink".into(),
                "release/deps/csr*.rlink".into(),
            ],
            instrumented_root_ir: ir.display().to_string(),
            rustc_link_metadata: rlink.display().to_string(),
            cargo_artifacts: "cargo-artifacts.jsonl".into(),
            linked_wasm_selection: None,
            linked_wasm: None,
        },
    )
}

pub fn retain_linked_wasm(
    target_release: &Path,
    manifest_path: &Path,
    retained_wasm: &Path,
) -> Result<()> {
    let wasm = select_one(target_release, "wasm")?;
    fs::copy(&wasm, retained_wasm)?;
    let mut manifest: IrManifest = read_json(manifest_path)?;
    manifest.linked_wasm_selection = Some(vec![
        "release/csr*.wasm".into(),
        "release/deps/csr*.wasm".into(),
    ]);
    manifest.linked_wasm = Some(wasm.display().to_string());
    write_json(manifest_path, &manifest)
}

pub fn assert_coverage(metadata_path: &Path) -> Result<()> {
    let metadata: CoverageMetadata = read_json(metadata_path)?;
    if metadata.result != "preserved" {
        bail!("coverage metadata was not retained: {}", metadata.result);
    }
    if !metadata.input.wasm_bindgen_metadata {
        bail!("manual link omitted __wasm_bindgen_unstable before wasm-bindgen");
    }
    Ok(())
}

pub fn write_instrumented_status(status_path: &Path, pipeline_exit: i32) -> Result<()> {
    let root = status_path
        .parent()
        .ok_or_else(|| anyhow!("status path has no parent"))?;
    let succeeded = pipeline_exit == 0;
    let served = succeeded.then(|| served_module(root)).transpose()?;
    let bundle = served.as_ref().map(|module| module.path.clone());
    let source_mappable_module = if succeeded {
        Some(SourceMappableModule {
            path: "instrumented/csr.wasm",
            sha256: sha256(root.join("instrumented/csr.wasm"))?,
            relationship_to_served_module: SERVED_RELATIONSHIP,
        })
    } else {
        None
    };
    write_json(
        status_path,
        &InstrumentedStatus {
            version: 3,
            outcome: if succeeded { "succeeded" } else { "failed" },
            pipeline_exit,
            module: succeeded.then_some("instrumented/csr.wasm"),
            bundle,
            served_module: served,
            source_mappable_module,
            source_identity: "source-identity.json",
            coverage_metadata: root
                .join("coverage-metadata.json")
                .is_file()
                .then_some("coverage-metadata.json"),
            toolchain_identity: root
                .join("toolchain-identity.json")
                .is_file()
                .then_some("toolchain-identity.json"),
            ir_manifest: root
                .join("instrumented/ir-manifest.json")
                .is_file()
                .then_some("instrumented/ir-manifest.json"),
            diagnostic_log: "pipeline.log",
        },
    )
}

pub fn write_baseline_status(status_path: &Path) -> Result<()> {
    write_json(
        status_path,
        &BaselineStatus {
            version: 1,
            outcome: "succeeded",
            served_module: served_module(
                status_path
                    .parent()
                    .ok_or_else(|| anyhow!("status path has no parent"))?,
            )?,
            unavoidable_deviations: [
                "omits -Cinstrument-coverage and minicov profiler runtime",
                "omits diagnostic-coverage feature and diagnostic browser exports",
            ],
        },
    )
}

fn select_one(target_release: &Path, extension: &str) -> Result<PathBuf> {
    let mut candidates = [target_release.to_path_buf(), target_release.join("deps")]
        .into_iter()
        .flat_map(|directory| fs::read_dir(directory).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("csr"))
                && path.extension().is_some_and(|suffix| suffix == extension)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    if candidates.len() != 1 {
        bail!("expected exactly one fresh CSR {extension}; found {candidates:?}");
    }
    Ok(candidates.pop().expect("length checked"))
}

fn served_module(root: &Path) -> Result<ServedModule> {
    let manifest: BundleManifest = read_json(&root.join("pkg/manifest.json"))?;
    let wasm = manifest
        .assets
        .into_iter()
        .find(|asset| asset.role.as_deref() == Some("wasm"))
        .ok_or_else(|| anyhow!("no wasm asset in CSR manifest"))?;
    Ok(ServedModule {
        sha256: sha256(root.join("pkg").join(&wasm.path))?,
        path: wasm.path,
    })
}

fn sha256(path: PathBuf) -> Result<String> {
    Ok(crate::digest::lowercase_hex(Sha256::digest(fs::read(
        path,
    )?)))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("reading JSON from {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(value)?))
        .with_context(|| format!("writing JSON to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rewrites_manifests_and_records_source_identity() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("client")).unwrap();
        fs::write(source.join("Cargo.toml"), "[patch.crates-io]\nfoo = 'x'\n").unwrap();
        fs::write(
            source.join("client/Cargo.toml"),
            "diagnostic-coverage = []\n",
        )
        .unwrap();
        let identity = temp.path().join("identity.json");
        prepare_source(
            &source,
            Path::new("/minicov"),
            Path::new("/runtime"),
            &identity,
            Path::new("/nix-source"),
        )
        .unwrap();
        assert!(
            fs::read_to_string(source.join("Cargo.toml"))
                .unwrap()
                .contains("minicov = { path = \"/minicov\" }")
        );
        assert!(
            fs::read_to_string(source.join("client/Cargo.toml"))
                .unwrap()
                .contains("diagnostic-coverage-runtime = { path = \"/runtime\", optional = true }")
        );
        assert_eq!(
            read_json::<serde_json::Value>(&identity).unwrap()["source_identity"]["value"].as_str(),
            Some("/nix-source")
        );
    }

    #[test]
    fn rejects_llvm_major_mismatch() {
        let temp = tempdir().unwrap();
        let rustc = temp.path().join("rustc");
        let clang = temp.path().join("clang");
        fs::write(&rustc, "LLVM version: 22.1.8\n").unwrap();
        fs::write(&clang, "clang version 21.0.0\n").unwrap();
        assert!(validate_llvm(&rustc, &clang).is_err());
    }

    #[test]
    fn discovers_and_prunes_to_the_single_linked_artifact() {
        let temp = tempdir().unwrap();
        let release = temp.path().join("release");
        fs::create_dir_all(release.join("deps")).unwrap();
        fs::write(release.join("csr.ll"), "ir").unwrap();
        fs::write(release.join("deps/csr.rlink"), "rlink").unwrap();
        let root_ir = temp.path().join("root-ir");
        let root_rlink = temp.path().join("root-rlink");
        let manifest = temp.path().join("manifest.json");
        discover_root(&release, &root_ir, &root_rlink, &manifest).unwrap();
        fs::write(release.join("csr.wasm"), "wasm").unwrap();
        let retained = temp.path().join("csr.wasm");
        retain_linked_wasm(&release, &manifest, &retained).unwrap();
        assert_eq!(fs::read(&retained).unwrap(), b"wasm");
        let linked_wasm = release.join("csr.wasm").display().to_string();
        assert_eq!(
            read_json::<serde_json::Value>(&manifest).unwrap()["linked_wasm"].as_str(),
            Some(linked_wasm.as_str())
        );
    }

    #[test]
    fn writes_instrumented_success_and_failure_statuses() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::create_dir_all(root.join("instrumented")).unwrap();
        fs::write(root.join("pkg/module.wasm"), "served").unwrap();
        fs::write(root.join("instrumented/csr.wasm"), "retained").unwrap();
        fs::write(
            root.join("pkg/manifest.json"),
            r#"{"assets":[{"path":"module.wasm","role":"wasm"}]}"#,
        )
        .unwrap();
        let status = root.join("status.json");
        write_instrumented_status(&status, 0).unwrap();
        let success: serde_json::Value = read_json(&status).unwrap();
        assert_eq!(success["outcome"], "succeeded");
        write_instrumented_status(&status, 7).unwrap();
        let failure: serde_json::Value = read_json(&status).unwrap();
        assert_eq!(failure["outcome"], "failed");
        assert_eq!(failure["pipeline_exit"], 7);
        assert!(failure["served_module"].is_null());
    }
}

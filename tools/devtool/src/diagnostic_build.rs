//! Typed artifact operations for the diagnostic CSR WebAssembly build.
//!
//! Nix owns process orchestration; this module owns every structured file and
//! filesystem selection that defines the diagnostic build's observable output.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PATH_RELATIONSHIP: &str = "The retained module is linked from LLVM IR compiled below compiled_source_prefix; the content-sensitive wasm output selected by pkg/.vite/manifest.json is its wasm-bindgen/wasm-opt derivative.";
const SERVED_RELATIONSHIP: &str = "input to wasm-bindgen and wasm-opt that produced the content-sensitive wasm output selected by pkg/.vite/manifest.json";

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

const COVERAGE_SECTIONS: [&str; 2] = ["__llvm_covfun", "__llvm_covmap"];

#[derive(Serialize)]
struct WasmBoundary {
    artifact: &'static str,
    coverage_sections: [bool; 2],
    wasm_bindgen_metadata: bool,
    exports: Vec<String>,
}

#[derive(Serialize)]
struct CoverageMetadataStatus {
    version: u8,
    required_sections: [SectionPresence; 2],
    input: WasmBoundary,
    after_wasm_bindgen: WasmBoundary,
    after_wasm_opt: WasmBoundary,
    result: &'static str,
}

#[derive(Serialize)]
struct SectionPresence {
    name: &'static str,
    present: bool,
}

#[derive(Serialize)]
struct ToolchainIdentity<'a> {
    version: u8,
    rustc: String,
    clang: String,
    llvm_profdata: String,
    llvm_cov: String,
    wasm_bindgen: String,
    wasm_opt: String,
    minicov: &'a str,
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
    let wasm_test_dependency = "wasm-bindgen-test = \"=0.3.78\"\n";
    if client.matches(wasm_test_dependency).count() != 1 {
        bail!(
            "copied client manifest must declare the pinned wasm-bindgen-test dev dependency exactly once"
        );
    }
    let client = client.replace(wasm_test_dependency, "");
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

pub fn write_coverage_artifacts(
    metadata_path: &Path,
    toolchain_identity_path: &Path,
    minicov_version: &str,
    input_wasm: &Path,
    after_wasm_bindgen: &Path,
    after_wasm_opt: &Path,
) -> Result<()> {
    let metadata = coverage_metadata_status(
        &fs::read(input_wasm).with_context(|| format!("reading {}", input_wasm.display()))?,
        &fs::read(after_wasm_bindgen)
            .with_context(|| format!("reading {}", after_wasm_bindgen.display()))?,
        &fs::read(after_wasm_opt)
            .with_context(|| format!("reading {}", after_wasm_opt.display()))?,
    )?;
    write_json(metadata_path, &metadata)?;
    let identity = ToolchainIdentity {
        version: 1,
        rustc: command_version("rustc", &["-Vv"])?,
        clang: command_version("clang", &["--version"])?,
        llvm_profdata: command_version("llvm-profdata", &["--version"])?,
        llvm_cov: command_version("llvm-cov", &["--version"])?,
        wasm_bindgen: command_version("wasm-bindgen", &["--version"])?,
        wasm_opt: command_version("wasm-opt", &["--version"])?,
        minicov: minicov_version,
    };
    write_json(toolchain_identity_path, &identity)
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
    // The pinned nightly's Cargo places root artifacts under build/<crate>/<hash>/out.
    // Also accept the established release/deps layout, still requiring one root.
    let mut directories = vec![target_release.to_path_buf(), target_release.join("deps")];
    if let Ok(builds) = fs::read_dir(target_release.join("build/csr")) {
        directories.extend(builds.flatten().map(|entry| entry.path().join("out")));
    }
    let mut candidates = directories
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

fn command_version(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("spawning {program} for diagnostic identity"))?;
    if !output.status.success() {
        bail!("{program} identity command failed ({})", output.status);
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("{program} identity command wrote non-UTF-8 stdout"))
}

fn read_u32_leb(bytes: &[u8], index: &mut usize, subject: &str) -> Result<u32> {
    let mut value = 0_u32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes
            .get(*index)
            .with_context(|| format!("truncated {subject}"))?;
        *index += 1;
        let bits = u32::from(byte & 0x7f);
        if shift == 28 && bits > 0x0f {
            bail!("{subject} overflows u32");
        }
        value |= bits << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("{subject} is overlong")
}

struct WasmSection<'a> {
    id: u8,
    payload: &'a [u8],
}

fn wasm_sections(wasm: &[u8]) -> Result<Vec<WasmSection<'_>>> {
    if !wasm.starts_with(b"\0asm\x01\0\0\0") {
        bail!("not a wasm version-1 module");
    }
    let mut sections = Vec::new();
    let mut index = 8;
    while index < wasm.len() {
        let id = *wasm.get(index).context("truncated wasm section id")?;
        index += 1;
        let length = read_u32_leb(wasm, &mut index, "wasm section length")? as usize;
        let end = index
            .checked_add(length)
            .context("wasm section length overflows module")?;
        let payload = wasm
            .get(index..end)
            .context("wasm section extends past module")?;
        sections.push(WasmSection { id, payload });
        index = end;
    }
    Ok(sections)
}

fn custom_section_name(section: &[u8]) -> Result<&[u8]> {
    let mut index = 0;
    let length = read_u32_leb(section, &mut index, "wasm custom-section name length")? as usize;
    let end = index
        .checked_add(length)
        .context("custom-section name length overflows section")?;
    section
        .get(index..end)
        .context("truncated wasm custom-section name")
}

fn coverage_section_presence(wasm: &[u8]) -> Result<[bool; 2]> {
    let mut found = [false; 2];
    for section in wasm_sections(wasm)? {
        if section.id != 0 {
            continue;
        }
        let name = custom_section_name(section.payload)?;
        for (index, required) in COVERAGE_SECTIONS.iter().enumerate() {
            found[index] |= name == required.as_bytes();
        }
    }
    Ok(found)
}

fn wasm_custom_section_present(wasm: &[u8], wanted: &str) -> Result<bool> {
    for section in wasm_sections(wasm)? {
        if section.id == 0 && custom_section_name(section.payload)? == wanted.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn wasm_exports(wasm: &[u8]) -> Result<Vec<String>> {
    let mut exports = Vec::new();
    for section in wasm_sections(wasm)? {
        if section.id != 7 {
            continue;
        }
        let mut index = 0;
        let count = read_u32_leb(section.payload, &mut index, "wasm export count")?;
        for _ in 0..count {
            let name_length =
                read_u32_leb(section.payload, &mut index, "wasm export name length")? as usize;
            let end = index
                .checked_add(name_length)
                .context("wasm export name length overflows section")?;
            let name = std::str::from_utf8(
                section
                    .payload
                    .get(index..end)
                    .context("truncated wasm export name")?,
            )
            .context("wasm export name is not UTF-8")?;
            exports.push(name.to_owned());
            index = end;
            index += 1; // export kind
            let _ = read_u32_leb(section.payload, &mut index, "wasm export index")?;
        }
        ensure!(
            index == section.payload.len(),
            "trailing data in wasm export section"
        );
    }
    Ok(exports)
}

fn wasm_boundary(artifact: &'static str, wasm: &[u8]) -> Result<WasmBoundary> {
    Ok(WasmBoundary {
        artifact,
        coverage_sections: coverage_section_presence(wasm)?,
        wasm_bindgen_metadata: wasm_custom_section_present(wasm, "__wasm_bindgen_unstable")?,
        exports: wasm_exports(wasm)?,
    })
}

fn coverage_metadata_status(
    input: &[u8],
    after_bindgen: &[u8],
    after_opt: &[u8],
) -> Result<CoverageMetadataStatus> {
    let input = wasm_boundary("csr.wasm", input)?;
    let after_wasm_bindgen = wasm_boundary("wasm-bindgen.wasm", after_bindgen)?;
    let after_wasm_opt = wasm_boundary("wasm-opt.wasm", after_opt)?;
    let result = if input.coverage_sections.iter().all(|present| *present) {
        if after_wasm_bindgen
            .coverage_sections
            .iter()
            .all(|present| *present)
            && after_wasm_opt
                .coverage_sections
                .iter()
                .all(|present| *present)
        {
            "preserved"
        } else {
            "lost-during-bundling"
        }
    } else {
        "missing-before-bundling"
    };
    Ok(CoverageMetadataStatus {
        version: 2,
        required_sections: [
            SectionPresence {
                name: COVERAGE_SECTIONS[0],
                present: after_wasm_opt.coverage_sections[0],
            },
            SectionPresence {
                name: COVERAGE_SECTIONS[1],
                present: after_wasm_opt.coverage_sections[1],
            },
        ],
        input,
        after_wasm_bindgen,
        after_wasm_opt,
        result,
    })
}

fn served_module(root: &Path) -> Result<ServedModule> {
    let wasm_path = crate::vite_manifest::wasm_output(&root.join("pkg"))?;
    Ok(ServedModule {
        sha256: sha256(root.join("pkg").join(&wasm_path))?,
        path: wasm_path,
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

    fn encode_leb(mut value: u32, output: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn wasm_with_sections(names: &[&str]) -> Vec<u8> {
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        for name in names {
            let mut payload = Vec::new();
            encode_leb(name.len() as u32, &mut payload);
            payload.extend_from_slice(name.as_bytes());
            payload.extend_from_slice(b"coverage");
            wasm.push(0);
            encode_leb(payload.len() as u32, &mut wasm);
            wasm.extend(payload);
        }
        wasm
    }

    #[test]
    fn diagnostic_coverage_metadata_preserves_sections_and_identifies_loss_boundary() {
        let complete = wasm_with_sections(&COVERAGE_SECTIONS);
        let status = coverage_metadata_status(&complete, &complete, &complete).unwrap();
        assert_eq!(status.result, "preserved");
        assert_eq!(status.input.coverage_sections, [true, true]);
        let lost = wasm_with_sections(&[COVERAGE_SECTIONS[0]]);
        let status = coverage_metadata_status(&complete, &complete, &lost).unwrap();
        assert_eq!(status.result, "lost-during-bundling");
        assert_eq!(status.after_wasm_opt.coverage_sections, [true, false]);
    }

    #[test]
    fn diagnostic_wasm_metadata_parser_rejects_truncated_sections() {
        let truncated = [b"\0asm\x01\0\0\0".as_slice(), &[0, 1]].concat();
        assert!(coverage_section_presence(&truncated).is_err());
        let truncated_export = [b"\0asm\x01\0\0\0".as_slice(), &[7, 1, 1]].concat();
        assert!(wasm_exports(&truncated_export).is_err());
    }

    #[test]
    fn rewrites_manifests_and_records_source_identity() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("client")).unwrap();
        fs::write(source.join("Cargo.toml"), "[patch.crates-io]\nfoo = 'x'\n").unwrap();
        fs::write(
            source.join("client/Cargo.toml"),
            "diagnostic-coverage = []\n[dev-dependencies]\nwasm-bindgen-test = \"=0.3.78\"\n",
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
        let client_manifest = fs::read_to_string(source.join("client/Cargo.toml")).unwrap();
        assert!(
            client_manifest
                .contains("diagnostic-coverage-runtime = { path = \"/runtime\", optional = true }")
        );
        assert!(!client_manifest.contains("wasm-bindgen-test"));
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
        fs::create_dir_all(release.join("build/csr/hash/out")).unwrap();
        fs::write(release.join("build/csr/hash/out/csr.ll"), "ir").unwrap();
        fs::write(release.join("build/csr/hash/out/csr.rlink"), "rlink").unwrap();
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
        fs::create_dir_all(root.join("pkg/.vite")).unwrap();
        fs::create_dir_all(root.join("pkg/assets")).unwrap();
        fs::create_dir_all(root.join("instrumented")).unwrap();
        fs::write(root.join("pkg/assets/csr_bg-hash.wasm"), b"served").unwrap();
        fs::write(root.join("instrumented/csr.wasm"), "retained").unwrap();
        fs::write(
            root.join("pkg/.vite/manifest.json"),
            r#"{"index.html":{"file":"assets/index.js","src":"index.html","isEntry":true},"csr_bg.wasm":{"file":"assets/csr_bg-hash.wasm","src":"csr_bg.wasm"}}"#,
        )
        .unwrap();
        let status = root.join("status.json");
        write_instrumented_status(&status, 0).unwrap();
        let success: serde_json::Value = read_json(&status).unwrap();
        assert_eq!(success["outcome"], "succeeded");
        assert_eq!(success["served_module"]["path"], "assets/csr_bg-hash.wasm");
        assert_eq!(
            success["served_module"]["sha256"],
            crate::digest::lowercase_hex(Sha256::digest(b"served"))
        );
        write_instrumented_status(&status, 7).unwrap();
        let failure: serde_json::Value = read_json(&status).unwrap();
        assert_eq!(failure["outcome"], "failed");
        assert_eq!(failure["pipeline_exit"], 7);
        assert!(failure["served_module"].is_null());
    }
}

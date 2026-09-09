//! Owns the on-guest WebAssembly coverage evidence lifecycle.
//!
//! The browser producer and this tool communicate only through the versioned
//! `status.json` artifact. Keeping its state transitions here prevents the Nix
//! test harness from becoming a second executable implementation.

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};
use coverage::wasm::{Artifact, BrowserStatus, Outcome, ServedModule, Stage};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const ROOT: &str = "/var/lib/jaunder/wasm-coverage";

#[derive(Deserialize)]
struct CsrStatus {
    outcome: String,
}

#[derive(Deserialize)]
struct Manifest {
    assets: Vec<ManifestAsset>,
}

#[derive(Deserialize)]
struct ManifestAsset {
    path: String,
    sha256: String,
    role: Option<String>,
}

#[derive(Deserialize)]
struct SourceIdentity {
    compilation_directory: String,
}

pub fn initialize() -> Result<()> {
    let root = Path::new(ROOT);
    let csr = required_path("JAUNDER_WASM_COVERAGE_CSR")?;
    let browser = required_env("JAUNDER_WASM_COVERAGE_BROWSER")?;
    initialize_at(root, &csr, &browser)
}

pub fn map(site_src: &Path) -> Result<()> {
    let root = Path::new(ROOT);
    let csr = required_path("JAUNDER_WASM_COVERAGE_CSR")?;
    let injected_failure = required_env("JAUNDER_WASM_COVERAGE_INJECT_FAILURE")?;
    map_at(root, &csr, &injected_failure, site_src)
}

pub fn finalize() -> Result<()> {
    finalize_at(
        Path::new(ROOT),
        &required_env("JAUNDER_WASM_COVERAGE_PLAYWRIGHT_EXIT")?,
    )
}

fn initialize_at(root: &Path, csr: &Path, browser: &str) -> Result<()> {
    fs::create_dir_all(root)?;
    let diagnostics = root.join("diagnostics/capture.log");
    fs::create_dir_all(diagnostics.parent().expect("diagnostics has parent"))?;

    let (module, served_module, structural) = match prepare_module(root, csr) {
        Ok(value) => value,
        Err(error) => {
            let module = root.join("module/unavailable.wasm");
            fs::create_dir_all(module.parent().expect("module has parent"))?;
            fs::write(&module, [])?;
            (module, None, Stage::failed(error.to_string()))
        }
    };
    fs::write(&diagnostics, "Playwright has not started\n")?;
    let mut artifacts = BTreeMap::new();
    artifacts.insert("module".into(), artifact_at(root, &module)?);
    artifacts.insert("diagnostics".into(), artifact_at(root, &diagnostics)?);
    write_status(
        root,
        &BrowserStatus {
            version: "v1".into(),
            requested_browser: browser.into(),
            actual_browser: "not-started".into(),
            csr_structural: structural,
            diagnostic_export: Stage::not_run("Playwright has not completed"),
            source_mapping: Stage::not_run("diagnostic export has not completed"),
            module_signature: None,
            toolchain_identity: None,
            served_module,
            artifacts,
        },
    )
}

fn prepare_module(root: &Path, csr: &Path) -> Result<(PathBuf, Option<ServedModule>, Stage)> {
    let csr_status: CsrStatus = read_json(&csr.join("status.json"))?;
    let manifest: Manifest = read_json(&csr.join("pkg/manifest.json"))?;
    let wasm = manifest
        .assets
        .into_iter()
        .find(|asset| asset.role.as_deref() == Some("wasm"))
        .ok_or_else(|| anyhow!("no wasm asset in CSR manifest"))?;
    let served_module = ServedModule {
        path: wasm.path.clone(),
        sha256: wasm.sha256,
    };
    let module = root.join("module").join(&wasm.path);
    fs::create_dir_all(module.parent().expect("module has parent"))?;
    fs::copy(csr.join("pkg").join(&wasm.path), &module)?;
    let structural = if csr_status.outcome == "succeeded"
        && artifact_at(root, &module)?.sha256 == served_module.sha256
    {
        Stage::passed()
    } else {
        Stage::failed("diagnostic CSR status or served module digest is invalid")
    };
    Ok((module, Some(served_module), structural))
}

fn map_at(root: &Path, csr: &Path, injected_failure: &str, site_src: &Path) -> Result<()> {
    let mut status = read_status(root)?;
    let diagnostics = root.join("diagnostics");
    fs::create_dir_all(&diagnostics)?;
    let mapping_log = diagnostics.join("mapping.log");

    if status.diagnostic_export.outcome != Outcome::Passed {
        let blocker = status
            .diagnostic_export
            .blocker
            .clone()
            .unwrap_or_else(|| "diagnostic export did not produce a profile".into());
        return mapping_skip(root, &mut status, &mapping_log, blocker);
    }
    if injected_failure == "mapping" {
        return mapping_fail(
            root,
            &mut status,
            &mapping_log,
            "injected mapping failure".into(),
        );
    }

    let identity: SourceIdentity = read_json(&csr.join("source-identity.json"))?;
    let mapped = root.join("mapped");
    fs::create_dir(&mapped)?;
    let profile = mapped.join("browser.profdata");
    let report = mapped.join("llvm-cov.txt");
    let profdata = Command::new(csr.join("tools/llvm-profdata"))
        .args(["merge", "-sparse"])
        .arg(root.join("profiles/browser.profraw"))
        .arg("-o")
        .arg(&profile)
        .output()
        .context("running llvm-profdata")?;
    if !profdata.status.success() {
        return mapping_fail(root, &mut status, &mapping_log, command_failure(&profdata));
    }
    let coverage = Command::new(csr.join("tools/llvm-cov"))
        .arg("show")
        .arg(csr.join("instrumented/csr.wasm"))
        .arg(format!("-instr-profile={}", profile.display()))
        .arg(format!(
            "-path-equivalence={},{}",
            identity.compilation_directory,
            site_src.display()
        ))
        .output()
        .context("running llvm-cov")?;
    fs::write(&report, &coverage.stdout)?;
    fs::write(
        &mapping_log,
        [&profdata.stderr[..], &coverage.stderr[..]].concat(),
    )?;
    if !coverage.status.success() {
        return mapping_fail(root, &mut status, &mapping_log, command_failure(&coverage));
    }
    if !has_executed_source_line(&String::from_utf8_lossy(&coverage.stdout)) {
        return mapping_fail(
            root,
            &mut status,
            &mapping_log,
            "llvm-cov report has no executed original Rust source line".into(),
        );
    }

    status.source_mapping = Stage::passed();
    status
        .artifacts
        .insert("profile_data".into(), artifact_at(root, &profile)?);
    status
        .artifacts
        .insert("mapped_report".into(), artifact_at(root, &report)?);
    status.artifacts.insert(
        "mapping_diagnostics".into(),
        artifact_at(root, &mapping_log)?,
    );
    write_status(root, &status)
}

fn mapping_skip(
    root: &Path,
    status: &mut BrowserStatus,
    mapping_log: &Path,
    detail: String,
) -> Result<()> {
    fs::write(mapping_log, format!("{detail}\n"))?;
    status.source_mapping = Stage::not_run(detail);
    status.artifacts.insert(
        "mapping_diagnostics".into(),
        artifact_at(root, mapping_log)?,
    );
    write_status(root, status)
}

fn mapping_fail(
    root: &Path,
    status: &mut BrowserStatus,
    mapping_log: &Path,
    mut detail: String,
) -> Result<()> {
    let report = root.join("mapped/llvm-cov.txt");
    let excerpt = if report.is_file() {
        format!(
            "\n--- llvm-cov report ---\n{}",
            fs::read_to_string(&report)?
        )
    } else {
        String::new()
    };
    match fs::remove_dir_all(root.join("mapped")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            detail = format!("{detail}; failed to remove partial mapping evidence: {error}")
        }
    }
    fs::write(mapping_log, format!("{detail}\n{excerpt}"))?;
    status.source_mapping = Stage::failed(detail);
    status.artifacts.insert(
        "mapping_diagnostics".into(),
        artifact_at(root, mapping_log)?,
    );
    write_status(root, status)?;
    bail!("coverage source mapping failed")
}

fn finalize_at(root: &Path, playwright_exit: &str) -> Result<()> {
    let mut status = read_status(root)?;
    if status.actual_browser != "not-started" {
        status.artifacts.insert(
            "playwright_diagnostics".into(),
            artifact_at(root, &root.join("diagnostics/playwright.log"))?,
        );
        return write_status(root, &status);
    }

    let capture_log = root.join("diagnostics/capture.log");
    if capture_log.exists() {
        fs::remove_file(capture_log)?;
    }
    for path in [root.join("profiles"), root.join("mapped")] {
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
    }
    let mapping_log = root.join("diagnostics/mapping.log");
    if mapping_log.exists() {
        fs::remove_file(mapping_log)?;
    }

    let blocker =
        format!("Playwright exited with status {playwright_exit} before coverage capture");
    status.actual_browser = "not-started".into();
    status.diagnostic_export = Stage::failed(blocker);
    status.source_mapping = Stage::not_run("diagnostic export did not run");
    status.module_signature = None;
    status.toolchain_identity = None;
    status.artifacts = BTreeMap::from([
        (
            "module".into(),
            artifact_at(root, &root.join(&status.artifacts["module"].path))?,
        ),
        (
            "diagnostics".into(),
            artifact_at(root, &root.join("diagnostics/playwright.log"))?,
        ),
    ]);
    write_status(root, &status)
}

fn read_status(root: &Path) -> Result<BrowserStatus> {
    read_json(&root.join("status.json"))
}

fn write_status(root: &Path, status: &BrowserStatus) -> Result<()> {
    fs::write(
        root.join("status.json"),
        format!("{}\n", serde_json::to_string_pretty(status)?),
    )?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("parsing {}", path.display()))
}

fn artifact_at(root: &Path, path: &Path) -> Result<Artifact> {
    Ok(Artifact {
        path: path
            .strip_prefix(root)
            .context("artifact lies outside coverage root")?
            .display()
            .to_string(),
        sha256: crate::digest::lowercase_hex(Sha256::digest(fs::read(path)?)),
    })
}

fn command_failure(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        output.status.to_string()
    } else {
        stderr
    }
}

fn has_executed_source_line(report: &str) -> bool {
    report.lines().any(|line| {
        let line = line.trim_start();
        let Some((line_number, rest)) = line.split_once('|') else {
            return false;
        };
        if line_number.is_empty()
            || !line_number
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return false;
        }
        let rest = rest.trim_start();
        let Some((count, _)) = rest.split_once('|') else {
            return false;
        };
        matches!(count.as_bytes(), [b'1'..=b'9', rest @ ..] if rest.iter().all(u8::is_ascii_digit))
    })
}

fn required_env(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing {name}"))
}
fn required_path(name: &str) -> Result<PathBuf> {
    Ok(required_env(name)?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn initialized_status() -> BrowserStatus {
        BrowserStatus {
            version: "v1".into(),
            requested_browser: "chromium".into(),
            actual_browser: "not-started".into(),
            csr_structural: Stage::passed(),
            diagnostic_export: Stage::not_run("Playwright has not completed"),
            source_mapping: Stage::not_run("diagnostic export has not completed"),
            module_signature: None,
            toolchain_identity: None,
            served_module: None,
            artifacts: BTreeMap::from([(
                "module".into(),
                Artifact {
                    path: "module/a.wasm".into(),
                    sha256: "unused".into(),
                },
            )]),
        }
    }

    #[test]
    fn mapping_skip_records_export_blocker() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        fs::create_dir_all(root.join("diagnostics")).unwrap();
        let mut status = initialized_status();
        status.diagnostic_export = Stage::failed("export failed");
        mapping_skip(
            root,
            &mut status,
            &root.join("diagnostics/mapping.log"),
            "export failed".into(),
        )
        .unwrap();
        let written = read_status(root).unwrap();
        assert_eq!(written.source_mapping, Stage::not_run("export failed"));
        assert_eq!(
            written.artifacts["mapping_diagnostics"].path,
            "diagnostics/mapping.log"
        );
    }

    #[test]
    fn mapping_failure_removes_partial_evidence_and_preserves_diagnostic() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        fs::create_dir_all(root.join("diagnostics")).unwrap();
        fs::create_dir_all(root.join("mapped")).unwrap();
        fs::write(root.join("mapped/llvm-cov.txt"), " 1| 1| hit\n").unwrap();
        let mut status = initialized_status();
        assert!(
            mapping_fail(
                root,
                &mut status,
                &root.join("diagnostics/mapping.log"),
                "bad map".into()
            )
            .is_err()
        );
        let written = read_status(root).unwrap();
        assert_eq!(written.source_mapping, Stage::failed("bad map"));
        assert!(!root.join("mapped").exists());
        assert!(
            fs::read_to_string(root.join("diagnostics/mapping.log"))
                .unwrap()
                .contains("llvm-cov report")
        );
    }

    #[test]
    fn finalizing_early_failure_discards_capture_only_artifacts() {
        let temporary = TempDir::new().unwrap();
        let root = temporary.path();
        fs::create_dir_all(root.join("module")).unwrap();
        fs::create_dir_all(root.join("diagnostics")).unwrap();
        fs::create_dir_all(root.join("profiles")).unwrap();
        fs::write(root.join("module/a.wasm"), "module").unwrap();
        fs::write(root.join("diagnostics/playwright.log"), "failed").unwrap();
        fs::write(root.join("diagnostics/capture.log"), "not started").unwrap();
        let status = initialized_status();
        write_status(root, &status).unwrap();
        finalize_at(root, "73").unwrap();
        let written = read_status(root).unwrap();
        assert_eq!(
            written.diagnostic_export,
            Stage::failed("Playwright exited with status 73 before coverage capture")
        );
        assert_eq!(
            written.source_mapping,
            Stage::not_run("diagnostic export did not run")
        );
        assert_eq!(
            written.artifacts.keys().collect::<Vec<_>>(),
            ["diagnostics", "module"]
        );
        assert!(!root.join("profiles").exists());
        assert!(!root.join("diagnostics/capture.log").exists());
    }
}

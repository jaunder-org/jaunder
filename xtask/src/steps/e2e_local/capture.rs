//! Jaunder's retained local-E2E capture policy.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;

pub(super) struct RetainedCapture {
    pub(super) run_dir: PathBuf,
    pub(super) capture_dir: PathBuf,
    pub(super) trace_path: PathBuf,
}

pub(super) fn allocate_retained_capture(
    root: &Path,
    browser: &str,
    test_support: &Path,
) -> anyhow::Result<RetainedCapture> {
    let retained_root = root.join(".xtask/e2e-local");
    fs::create_dir_all(&retained_root)
        .with_context(|| format!("creating {}", retained_root.display()))?;
    let run_dir = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&retained_root)
        .context("creating unique retained e2e-local run directory")?
        .keep();
    let capture_dir = run_dir.join(browser).join("capture");
    let output = Command::new(test_support)
        .args(["capture-path", "otel"])
        .env("JAUNDER_CAPTURE_DIR", &capture_dir)
        .env_remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .output()
        .with_context(|| {
            format!(
                "resolving OTel capture path with {}",
                test_support.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "test-support capture-path otel failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let trace_path = PathBuf::from(
        String::from_utf8(output.stdout)
            .context("test-support capture path was not UTF-8")?
            .trim(),
    );
    anyhow::ensure!(
        trace_path.parent() == Some(capture_dir.as_path()),
        "test-support returned OTel capture path outside {}: {}",
        capture_dir.display(),
        trace_path.display()
    );
    Ok(RetainedCapture {
        run_dir,
        capture_dir,
        trace_path,
    })
}

pub(super) fn finalize_capture(retained: &RetainedCapture, source: &Path) -> anyhow::Result<bool> {
    copy_capture_directory(source, &retained.capture_dir)?;
    if retained.trace_path.is_file() {
        println!("e2e-local trace: {}", retained.trace_path.display());
        Ok(true)
    } else {
        println!(
            "e2e-local capture retained at {}; expected trace missing: {}",
            retained.run_dir.display(),
            retained.trace_path.display()
        );
        Ok(false)
    }
}

fn copy_capture_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("reading {}", source.display()))? {
        let entry = entry.with_context(|| format!("reading {}", source.display()))?;
        let destination_entry = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading {}", entry.path().display()))?;
        if file_type.is_dir() {
            copy_capture_directory(&entry.path(), &destination_entry)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &destination_entry).with_context(|| {
                format!(
                    "copying {} to {}",
                    entry.path().display(),
                    destination_entry.display()
                )
            })?;
        } else {
            anyhow::bail!("unsupported capture entry {}", entry.path().display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalization_copies_diagnostics_before_reporting_trace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let source = tempfile::tempdir().expect("source");
        let run_dir = workspace.path().join("run");
        let retained = RetainedCapture {
            capture_dir: run_dir.join("capture"),
            trace_path: run_dir.join("capture/otel-traces.jsonl"),
            run_dir,
        };
        fs::create_dir_all(source.path().join("nested")).expect("nested diagnostics");
        fs::write(source.path().join("otel-traces.jsonl"), "{}\n").expect("trace");
        fs::write(source.path().join("nested/collector.log"), "diagnostic").expect("log");

        assert!(finalize_capture(&retained, source.path()).expect("finalize"));
        assert_eq!(
            fs::read_to_string(retained.capture_dir.join("nested/collector.log"))
                .expect("copied log"),
            "diagnostic"
        );
    }

    #[test]
    fn finalization_reports_missing_trace_after_retaining_directory() {
        let workspace = tempfile::tempdir().expect("workspace");
        let source = tempfile::tempdir().expect("source");
        let run_dir = workspace.path().join("run");
        let retained = RetainedCapture {
            capture_dir: run_dir.join("capture"),
            trace_path: run_dir.join("capture/otel-traces.jsonl"),
            run_dir,
        };
        fs::write(source.path().join("collector.log"), "diagnostic").expect("log");

        assert!(!finalize_capture(&retained, source.path()).expect("finalize"));
        assert!(retained.capture_dir.join("collector.log").is_file());
    }
}

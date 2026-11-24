//! `trace-analysis run` — build the e2e VM checks and analyze their traces.
//!
//! Host orchestrator (ADR-0028): nix-builds the `{sqlite,postgres}×{chromium,firefox}`
//! e2e checks (or their `-single-worker` package variants), extracts each combo's
//! `capture/otel-traces.jsonl` from its `capture-<backend>.tar.gz` bundle (#332 — the
//! trace rides the capture dir, not a standalone artifact), and hands the files to
//! the in-process `traces::analyze` / `render` seam. The pure helpers below are
//! unit-tested; the nix/filesystem I/O in `collect_trace_files` is manual.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use flate2::read::GzDecoder;
use host::capture::Stream;
use tempfile::TempDir;

use crate::e2e::{E2eBackend, E2eBrowser};
use crate::nix_build::build_out_path;

/// Both backends are always built, regardless of `--browser` (matches the script).
const BACKENDS: [E2eBackend; 2] = [E2eBackend::Sqlite, E2eBackend::Postgres];

/// The trace member inside every `capture-<backend>.tar.gz` — the collector writes
/// `capture/otel-traces.jsonl` under the capture dir, and the tarball is rooted at the
/// capture dir's parent (`tar … -C /var/lib/jaunder capture`).
pub const TRACE_MEMBER: &str = "capture/otel-traces.jsonl";

/// The flake attr for one e2e combo. `single_worker` → the `packages…-single-worker`
/// variant, otherwise the gate check →
/// the `checks…` variant (e.g. `checks.x86_64-linux.e2e-sqlite-chromium`,
/// `packages.x86_64-linux.e2e-postgres-firefox-single-worker`).
pub fn e2e_attr(backend: E2eBackend, browser: E2eBrowser, single_worker: bool) -> String {
    let ns = if single_worker { "packages" } else { "checks" };
    let suffix = if single_worker { "-single-worker" } else { "" };
    format!(
        "{ns}.x86_64-linux.e2e-{}-{}{suffix}",
        backend.as_str(),
        browser.as_str()
    )
}

/// The capture bundle inside a built e2e check's store path:
/// `<out>/capture-<backend>.tar.gz` (holds `capture/otel-traces.jsonl`, #332).
pub fn capture_tarball_path(out: &str, backend: E2eBackend) -> PathBuf {
    PathBuf::from(out).join(format!("capture-{}.tar.gz", backend.as_str()))
}

/// The browsers to build: just `--browser` if given, else both.
pub fn browsers(browser: Option<E2eBrowser>) -> Vec<E2eBrowser> {
    match browser {
        Some(b) => vec![b],
        None => vec![E2eBrowser::Chromium, E2eBrowser::Firefox],
    }
}

/// Build every combo (both backends × the selected browsers), extract each
/// `capture/otel-traces.jsonl` from its `capture-<backend>.tar.gz` bundle to a
/// per-combo file in one `TempDir`, and collect the extracted files — erroring
/// (naming the tarball) when one is absent. The returned `TempDir` guards the
/// extracted files and MUST outlive analysis (the caller binds it); the per-combo
/// file name (`<backend>-<browser>-otel-traces.jsonl`) disambiguates the shared inner
/// member across combos. nix/filesystem I/O — not unit-tested; exercised by a manual
/// run. Iteration is backends-outer, browsers-inner, matching the script's file order.
pub fn collect_trace_files(
    single_worker: bool,
    browser: Option<E2eBrowser>,
) -> Result<(TempDir, Vec<PathBuf>, Vec<LabeledReport>)> {
    let tmp = TempDir::new()?;
    let browsers = browsers(browser);
    let mut files = Vec::new();
    let mut reports: Vec<LabeledReport> = Vec::new();
    for backend in BACKENDS {
        for &browser in &browsers {
            let attr = e2e_attr(backend, browser, single_worker);
            let out = build_out_path(&attr)?;
            let tarball = capture_tarball_path(&out, backend);
            ensure!(
                tarball.exists(),
                "capture tarball not found: {}",
                tarball.display()
            );
            let dest = tmp.path().join(format!(
                "{}-{}-otel-traces.jsonl",
                backend.as_str(),
                browser.as_str()
            ));
            extract_trace(&tarball, &dest)?;
            // `read_spans` labels every span with `path.display()`, so the
            // report pairing below must key on exactly that string.
            let source = dest.display().to_string();
            files.push(dest);

            // The Playwright report sits beside the capture in the same store
            // path. It is the span-coverage section's denominator (#794).
            //
            // Paired with THIS combo's trace-file name, which is what
            // `Span.source` carries. Without the pairing, sqlite's and postgres's
            // reports collide — `projectName` is the browser and names no
            // backend, so both combos yield the same (test, project, retry) key
            // with different durations.
            //
            // Deliberately NOT an error when absent: a pre-#794 check output has
            // no reason to carry one, and the coverage section degrades to a
            // stated note rather than taking the whole run down.
            let report = playwright_report_path(&out, backend);
            if report.exists() {
                reports.push((source, report));
            }
        }
    }
    Ok((tmp, files, reports))
}

/// One combo's Playwright report, paired with the `Span.source` label of the
/// trace file it belongs to.
///
/// The pairing is load-bearing, not bookkeeping: `projectName` is the browser
/// and names no backend, so sqlite's and postgres's reports produce identical
/// `(test, project, retry)` keys with different durations. See
/// [`super::report::ReportedDurations`].
pub type LabeledReport = (String, PathBuf);

/// Playwright's `json` reporter output inside a built e2e check's store path:
/// `<out>/playwright-report-<backend>.json`.
pub fn playwright_report_path(out: &str, backend: E2eBackend) -> PathBuf {
    PathBuf::from(out).join(format!("playwright-report-{}.json", backend.as_str()))
}

/// The capture bundle's root directory (`tar … -C /var/lib/jaunder capture`).
const CAPTURE_ROOT: &str = "capture";

/// One server instance's WARN-and-above diagnostic log inside a capture bundle:
/// each Playwright worker's server writes its own instance subdirectory of the
/// capture (#2076). It may legitimately be empty: a clean run warns about
/// nothing.
pub fn instance_diag_member(index: usize) -> String {
    format!(
        "{CAPTURE_ROOT}/{}/{}",
        host::capture::instance_directory(index),
        Stream::Diag.filename()
    )
}

/// The instance whose diagnostic log `member` is, the inverse of
/// [`instance_diag_member`].
fn instance_diag_index(member: &str) -> Option<usize> {
    let (root, rest) = member.split_once('/')?;
    let (directory, file) = rest.split_once('/')?;
    if root != CAPTURE_ROOT || file != Stream::Diag.filename() {
        return None;
    }
    host::capture::instance_index(directory)
}

/// Read the single required `member` from a capture archive.
pub fn read_capture_member(tarball: &Path, member: &str) -> Result<String> {
    read_capture_members(tarball, |path| path == member)?
        .remove(member)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "capture member `{member}` is missing from {}",
                tarball.display()
            )
        })
}

/// Read every server instance's `diag.log`, in instance order. The instances
/// must be numbered contiguously from 0, so a lost instance's log cannot pass
/// unnoticed, and at least one must exist.
pub fn read_instance_diag_logs(tarball: &Path) -> Result<Vec<String>> {
    let mut logs = read_capture_members(tarball, |path| instance_diag_index(path).is_some())?;
    ensure!(
        !logs.is_empty(),
        "capture has no `{}` member: {}",
        instance_diag_member(0),
        tarball.display()
    );
    (0..logs.len())
        .map(|index| {
            let member = instance_diag_member(index);
            logs.remove(&member).ok_or_else(|| {
                anyhow::anyhow!(
                    "capture member `{member}` is missing from {}",
                    tarball.display()
                )
            })
        })
        .collect()
}

/// Read every regular-file member `select`ed by its path. Every archive
/// consumer uses this seam so malformed evidence has the same ambiguous,
/// file-type, and UTF-8 failure modes.
fn read_capture_members(
    tarball: &Path,
    select: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, String>> {
    let file =
        File::open(tarball).with_context(|| format!("opening capture {}", tarball.display()))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut found = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(member) = entry
            .path()?
            .to_str()
            .filter(|path| select(path))
            .map(str::to_owned)
        else {
            continue;
        };
        ensure!(
            entry.header().entry_type().is_file(),
            "capture member `{member}` is not a regular file"
        );
        ensure!(
            !found.contains_key(&member),
            "capture has ambiguous member `{member}`"
        );
        let mut contents = String::new();
        entry
            .read_to_string(&mut contents)
            .with_context(|| format!("reading capture member `{member}`"))?;
        found.insert(member, contents);
    }
    Ok(found)
}

/// Read the capture's trace, which unlike other members must not be empty.
pub fn read_trace_member(tarball: &Path) -> Result<String> {
    let trace = read_capture_member(tarball, TRACE_MEMBER)?;
    ensure!(
        !trace.trim().is_empty(),
        "capture member `{TRACE_MEMBER}` is empty"
    );
    Ok(trace)
}

/// Extract the single `capture/otel-traces.jsonl` member of a `capture-*.tar.gz`
/// bundle to `dest`.
fn extract_trace(tarball: &Path, dest: &Path) -> Result<()> {
    let trace = read_trace_member(tarball)?;
    std::fs::write(dest, trace)
        .with_context(|| format!("writing extracted trace {}", dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_capture(path: &Path, entries: &[(&str, &[u8], tar::EntryType)]) {
        let file = File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for (name, content, entry_type) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*entry_type);
            header.set_size(content.len().try_into().unwrap());
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, *name, *content).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn trace_member_reader_rejects_incomplete_or_ambiguous_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let capture = temp.path().join("capture.tar.gz");

        write_capture(&capture, &[]);
        assert!(
            read_trace_member(&capture)
                .unwrap_err()
                .to_string()
                .contains("missing")
        );

        write_capture(&capture, &[(TRACE_MEMBER, b"", tar::EntryType::Regular)]);
        assert!(
            read_trace_member(&capture)
                .unwrap_err()
                .to_string()
                .contains("empty")
        );

        write_capture(
            &capture,
            &[(TRACE_MEMBER, b"trace", tar::EntryType::Directory)],
        );
        assert!(
            read_trace_member(&capture)
                .unwrap_err()
                .to_string()
                .contains("regular")
        );

        write_capture(
            &capture,
            &[(TRACE_MEMBER, b"\xff", tar::EntryType::Regular)],
        );
        assert!(read_trace_member(&capture).is_err());

        write_capture(
            &capture,
            &[
                (TRACE_MEMBER, b"one", tar::EntryType::Regular),
                (TRACE_MEMBER, b"two", tar::EntryType::Regular),
            ],
        );
        assert!(
            read_trace_member(&capture)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
    }

    #[test]
    fn capture_member_reader_accepts_an_empty_member() {
        let temp = tempfile::tempdir().unwrap();
        let capture = temp.path().join("capture.tar.gz");
        let member = instance_diag_member(0);
        write_capture(&capture, &[(member.as_str(), b"", tar::EntryType::Regular)]);
        assert_eq!(read_capture_member(&capture, &member).unwrap(), "");
    }

    #[test]
    fn instance_diag_logs_read_every_instance_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let capture = temp.path().join("capture.tar.gz");
        let (first, second) = (instance_diag_member(0), instance_diag_member(1));
        write_capture(
            &capture,
            &[
                (second.as_str(), b"two", tar::EntryType::Regular),
                (TRACE_MEMBER, b"trace", tar::EntryType::Regular),
                ("capture/diag.log", b"unscoped", tar::EntryType::Regular),
                (first.as_str(), b"", tar::EntryType::Regular),
            ],
        );
        assert_eq!(read_instance_diag_logs(&capture).unwrap(), ["", "two"]);
    }

    #[test]
    fn instance_diag_logs_reject_missing_or_gapped_instances() {
        let temp = tempfile::tempdir().unwrap();
        let capture = temp.path().join("capture.tar.gz");
        write_capture(
            &capture,
            &[(TRACE_MEMBER, b"trace", tar::EntryType::Regular)],
        );
        let error = read_instance_diag_logs(&capture).unwrap_err().to_string();
        assert!(error.contains("capture/instance-0/diag.log"), "{error}");

        let third = instance_diag_member(2);
        write_capture(
            &capture,
            &[
                (
                    instance_diag_member(0).as_str(),
                    b"",
                    tar::EntryType::Regular,
                ),
                (third.as_str(), b"", tar::EntryType::Regular),
            ],
        );
        let error = read_instance_diag_logs(&capture).unwrap_err().to_string();
        assert!(error.contains("capture/instance-1/diag.log"), "{error}");
    }

    #[test]
    fn e2e_attr_gate_and_single_worker() {
        assert_eq!(
            e2e_attr(E2eBackend::Sqlite, E2eBrowser::Chromium, false),
            "checks.x86_64-linux.e2e-sqlite-chromium"
        );
        assert_eq!(
            e2e_attr(E2eBackend::Postgres, E2eBrowser::Firefox, true),
            "packages.x86_64-linux.e2e-postgres-firefox-single-worker"
        );
    }

    #[test]
    fn capture_tarball_path_shape() {
        assert_eq!(
            capture_tarball_path("/nix/store/x", E2eBackend::Sqlite),
            PathBuf::from("/nix/store/x/capture-sqlite.tar.gz")
        );
    }

    #[test]
    fn browsers_one_or_both() {
        assert_eq!(
            browsers(Some(E2eBrowser::Firefox)),
            vec![E2eBrowser::Firefox]
        );
        assert_eq!(
            browsers(None),
            vec![E2eBrowser::Chromium, E2eBrowser::Firefox]
        );
    }
}

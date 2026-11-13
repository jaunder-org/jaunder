//! Filesystem edges of the flow-coverage gate (#681): the syn inventory, the
//! committed snapshot, and reading a capture bundle.
//!
//! **Everything here fails closed.** A missing, empty, or unparseable capture is
//! an error, never "no uncovered fns" — a silent pass would make the whole
//! mechanism dishonest, since the failure mode it is guarding against and the
//! failure mode of its own plumbing would look identical.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{Coverage, Snapshot, extract, render};
use crate::server_fns::ServerFn;
use host_proof::traces::parse::{Filters, parse_spans};

#[cfg(test)]
pub use repository_policy::source_inventory::read_snapshot;
pub use repository_policy::source_inventory::{SNAPSHOT_PATH, WEB_SRC, inventory};

/// Derive coverage from raw OTLP JSONL. Empty or unparseable content is an error,
/// not an empty [`Coverage`].
pub fn coverage_from_jsonl(jsonl: &str, inventory: &[ServerFn]) -> Result<Coverage> {
    let spans = parse_spans(jsonl, &Filters::default(), "capture")
        .context("parsing the e2e capture's otel-traces.jsonl")?;
    if spans.is_empty() {
        bail!(
            "the e2e capture contains no spans — refusing to treat an empty capture as \
             full coverage; re-run `just e2e-producer sqlite-chromium-unsplit`"
        );
    }
    Ok(extract(&spans, inventory))
}

/// Derive coverage from a `capture-*.tar.gz` bundle, reusing the trace extractor
/// that `traces run` already uses rather than a second implementation.
pub fn coverage_from_capture(tarball: &Path, inventory: &[ServerFn]) -> Result<Coverage> {
    if !tarball.exists() {
        bail!(
            "no e2e capture at {} — run `just e2e-producer sqlite-chromium-unsplit` first (coverage is \
             derived from that combo's traces, per the spec's D6)",
            tarball.display()
        );
    }
    let tmp = tempfile::tempdir().context("creating a temp dir for the extracted trace")?;
    let dest = tmp.path().join("otel-traces.jsonl");
    host_proof::traces::run::extract_trace(tarball, &dest)
        .with_context(|| format!("extracting the otel trace from {}", tarball.display()))?;
    let jsonl = std::fs::read_to_string(&dest).context("reading the extracted otel trace")?;
    coverage_from_jsonl(&jsonl, inventory)
}

/// Render and write the generated coverage snapshot, creating
/// `docs/coverage/` if needed.
pub fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, render(snapshot)?).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    fn empty_inventory() -> Vec<ServerFn> {
        Vec::new()
    }

    fn one_entry_coverage() -> Coverage {
        Coverage {
            covered: BTreeMap::from([(
                "posts::create".to_string(),
                BTreeSet::from(["a test".to_string()]),
            )]),
            orphans: BTreeMap::new(),
        }
    }

    #[test]
    fn missing_capture_fails_closed() {
        let err =
            coverage_from_capture(Path::new("/nonexistent-capture.tar.gz"), &empty_inventory())
                .unwrap_err();
        assert!(err.to_string().contains("capture"), "{err}");
    }

    #[test]
    fn empty_capture_fails_closed_rather_than_reporting_full_coverage() {
        let err = coverage_from_jsonl("", &empty_inventory()).unwrap_err();
        assert!(err.to_string().contains("no spans"), "{err}");
    }

    #[test]
    fn unparseable_capture_fails_closed() {
        assert!(coverage_from_jsonl("{not json\n", &empty_inventory()).is_err());
    }

    #[test]
    fn whitespace_only_capture_fails_closed() {
        assert!(coverage_from_jsonl("\n\n", &empty_inventory()).is_err());
    }

    #[test]
    fn missing_snapshot_error_names_the_regenerate_command() {
        let err = read_snapshot(Path::new("/nonexistent-snapshot.json")).unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains(super::super::REGENERATE_CMD), "{chain}");
    }

    #[test]
    fn write_snapshot_creates_the_directory_and_renders_stably() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nested").join("snapshot.json");
        let snapshot = one_entry_coverage().into_snapshot();
        write_snapshot(&path, &snapshot).expect("writes");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            render(&snapshot).expect("renders")
        );
    }
}

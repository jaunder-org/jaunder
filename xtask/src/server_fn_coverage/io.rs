//! Filesystem edges of the flow-coverage gate (#681): the syn inventory, the
//! committed artifacts, and reading a capture bundle.
//!
//! **Everything here fails closed.** A missing, empty, or unparseable capture is
//! an error, never "no uncovered fns" — a silent pass would make the whole
//! mechanism dishonest, since the failure mode it is guarding against and the
//! failure mode of its own plumbing would look identical.

use std::path::Path;

use anyhow::{bail, Context, Result};

use super::{extract, render, AllowlistEntry, Coverage, Snapshot};
use crate::files;
use crate::server_fns::{module_path_of, server_fns_in, ServerFn};
use crate::traces::parse::{parse_spans, Filters};

/// The `web` crate source root, scanned for the `#[server]` inventory.
pub const WEB_SRC: &str = "web/src";
/// The committed, generated coverage snapshot.
pub const SNAPSHOT_PATH: &str = "docs/coverage/server-fns.json";
/// The committed, hand-maintained allowlist.
pub const ALLOWLIST_PATH: &str = "docs/coverage/server-fns-allowlist.json";
/// Where `cargo xtask e2e sqlite chromium` lifts the authoritative capture.
pub const CAPTURE_PATH: &str = ".xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz";

/// Every `#[server]` fn under `web/src`, sorted by qualified name. A file that cannot be
/// enumerated is a hard error: a file we cannot read could hide a fn, and an
/// under-counted inventory is exactly a false pass.
pub fn inventory(root: &Path) -> Result<Vec<ServerFn>> {
    let files = files::with_extension(root, "rs")
        .with_context(|| format!("scanning {} for #[server] fns", root.display()))?;

    let mut out = Vec::new();
    for path in &files {
        let src =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let rel = path.strip_prefix(root).unwrap_or(path);
        match server_fns_in(&src, &module_path_of(rel)) {
            Ok(fns) => out.extend(fns),
            Err(msg) => bail!("{}: {msg}", path.display()),
        }
    }
    // By the coverage key, not the ident: idents collide across verticals (#684),
    // so an ident sort leaves the order of a collision set up to directory walk
    // order.
    out.sort_by_key(ServerFn::qualified);
    Ok(out)
}

/// Derive coverage from raw OTLP JSONL. Empty or unparseable content is an error,
/// not an empty [`Coverage`].
pub fn coverage_from_jsonl(jsonl: &str, inventory: &[ServerFn]) -> Result<Coverage> {
    let spans = parse_spans(jsonl, &Filters::default(), "capture")
        .context("parsing the e2e capture's otel-traces.jsonl")?;
    if spans.is_empty() {
        bail!(
            "the e2e capture contains no spans — refusing to treat an empty capture as \
             full coverage; re-run `cargo xtask e2e sqlite chromium`"
        );
    }
    Ok(extract(&spans, inventory))
}

/// Derive coverage from a `capture-*.tar.gz` bundle, reusing the trace extractor
/// that `traces run` already uses rather than a second implementation.
pub fn coverage_from_capture(tarball: &Path, inventory: &[ServerFn]) -> Result<Coverage> {
    if !tarball.exists() {
        bail!(
            "no e2e capture at {} — run `cargo xtask e2e sqlite chromium` first (coverage is \
             derived from that combo's traces, per the spec's D6)",
            tarball.display()
        );
    }
    let tmp = tempfile::tempdir().context("creating a temp dir for the extracted trace")?;
    let dest = tmp.path().join("otel-traces.jsonl");
    crate::traces::run::extract_trace(tarball, &dest)
        .with_context(|| format!("extracting the otel trace from {}", tarball.display()))?;
    let jsonl = std::fs::read_to_string(&dest).context("reading the extracted otel trace")?;
    coverage_from_jsonl(&jsonl, inventory)
}

/// The committed snapshot, or an error naming the remedy when it is absent.
pub fn read_snapshot(path: &Path) -> Result<Snapshot> {
    let raw = std::fs::read_to_string(path).with_context(|| {
        format!(
            "reading {} — if it does not exist yet, generate it with `{}`",
            path.display(),
            super::REGENERATE_CMD
        )
    })?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

/// The committed allowlist. A missing file is an empty allowlist — that is the
/// strict reading (nothing is excused), not a lenient one.
pub fn read_allowlist(path: &Path) -> Result<Vec<AllowlistEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

/// Write the snapshot to `path`, creating `docs/coverage/` if needed.
pub fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, render(snapshot)?).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_inventory() -> Vec<ServerFn> {
        Vec::new()
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
    fn missing_allowlist_is_empty_not_an_error() {
        let list = read_allowlist(Path::new("/nonexistent-allowlist.json")).expect("ok");
        assert!(list.is_empty());
    }

    #[test]
    fn missing_snapshot_error_names_the_regenerate_command() {
        let err = read_snapshot(Path::new("/nonexistent-snapshot.json")).unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains(super::super::REGENERATE_CMD), "{chain}");
    }
}

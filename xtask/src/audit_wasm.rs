//! `cargo xtask audit-wasm` — measure frontend bundle download weight.
//!
//! Host-side analysis tool (ADR-0028): it shells out to `nix build .#site` and
//! sizes the built artifacts, so it lives in `xtask` (the host analyzer), not
//! `devtool` (the in-sandbox producer). The pure size/format helpers are split
//! out and unit-tested; the `nix`/filesystem I/O lives in `run`/`resolve_site_path`.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::nix_build;

#[derive(Debug, Serialize)]
pub struct AuditReport {
    pub site_path: String,
    pub artifacts: Vec<ArtifactMetrics>,
}

#[derive(Debug, Serialize)]
pub struct ArtifactMetrics {
    pub path: String,
    pub raw_bytes: u64,
    pub gzip_bytes: u64,
    pub brotli_bytes: u64,
}

/// Human-readable byte size. Mirrors the old Node script's rounding exactly:
/// whole numbers for bytes and for any value ≥ 10 in its unit, one decimal
/// otherwise — so the rendered size table stays comparable with the old script.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let decimals = if value >= 10.0 || unit == 0 { 0 } else { 1 };
    format!("{value:.decimals$} {}", UNITS[unit])
}

/// gzip size at level 9 (`Z_BEST_COMPRESSION`), matching the script. Absolute
/// byte counts may differ by a few bytes from Node's `zlib` backend (`flate2`
/// uses `miniz_oxide`); the compression *parameters* — what governs the trend
/// this tool tracks — are identical.
pub fn gzip_size(bytes: &[u8]) -> u64 {
    use flate2::{write::GzEncoder, Compression};
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    enc.write_all(bytes)
        .expect("gzip write to Vec is infallible");
    enc.finish()
        .expect("gzip finish to Vec is infallible")
        .len() as u64
}

/// brotli size at quality 11, window 22 — the script set only quality 11 and
/// left the window at brotli's default (22).
pub fn brotli_size(bytes: &[u8]) -> u64 {
    let mut out = Vec::new();
    {
        let mut w = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
        w.write_all(bytes)
            .expect("brotli write to Vec is infallible");
    }
    out.len() as u64
}

/// The human size table: a header, the site store path, then one right-aligned
/// raw/gzip/brotli row per artifact named relative to the site path.
pub fn render_table(report: &AuditReport) -> String {
    let mut s = String::new();
    s.push_str("WASM bundle audit\n");
    s.push_str(&format!("site output: {}\n", report.site_path));
    s.push('\n');
    s.push_str("artifact          raw        gzip       brotli\n");
    for row in &report.artifacts {
        let name = Path::new(&row.path)
            .strip_prefix(&report.site_path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| row.path.clone());
        s.push_str(&format!(
            "{:<16}  {:>9}  {:>9}  {:>9}\n",
            name,
            format_bytes(row.raw_bytes),
            format_bytes(row.gzip_bytes),
            format_bytes(row.brotli_bytes),
        ));
    }
    s
}

/// Resolve the `.#site` output to audit. Returns `explicit` verbatim when set
/// (audit a prebuilt store path, e.g. in CI or while iterating); otherwise runs
/// the deterministic `nix build .#site` and parses its store path (shared
/// [`nix_build::build_out_path`]).
pub fn resolve_site_path(explicit: Option<&str>) -> Result<String> {
    match explicit {
        Some(p) => Ok(p.to_string()),
        None => nix_build::build_out_path("site"),
    }
}

/// The substring between the first occurrence of `marker` (a prefix ending at an
/// opening `"`) and the next `"` — the URL a boot-script directive quotes. `None`
/// if the marker or its closing quote is absent.
fn quoted_after(haystack: &str, marker: &str) -> Option<String> {
    let start = haystack.find(marker)? + marker.len();
    let rest = &haystack[start..];
    rest.find('"').map(|end| rest[..end].to_string())
}

/// The `pkg/`-relative artifacts the CSR shell actually boots, read from the
/// site's `index.html`: the wasm the boot passes to `init` (`init("…")`) and the
/// JS module it imports (`import init from "…"`). Deriving the audit's targets
/// from the shell — rather than a hard-coded name — is what makes this a real
/// guard: a wasm the shell references but the build never emitted (issue #234)
/// becomes a missing artifact here, instead of a silent 404 in the browser.
/// Rejects an arg-less `init()` (no explicit URL → wasm-bindgen's `_bg` default,
/// the #234 regression).
fn shell_boot_artifacts(index_html: &str) -> Result<Vec<String>> {
    let wasm = quoted_after(index_html, "init(\"").context(
        "index.html boot script has no explicit `init(\"…\")` wasm URL \
         (arg-less init() falls back to wasm-bindgen's _bg default — issue #234)",
    )?;
    let js = quoted_after(index_html, "import init from \"")
        .context("index.html has no `import init from \"…\"` module URL")?;
    Ok([wasm, js]
        .into_iter()
        .map(|url| url.trim_start_matches('/').to_string())
        .collect())
}

/// The CSR SPA shell the server embeds and serves (#239). Audited from source — it
/// is no longer copied into the built site — against the emitted bundle. xtask is
/// rebuilt from the live tree every run, so this stays current. Path depth:
/// `audit_wasm.rs` is at `xtask/src/`, so `../../` reaches the repo root.
const SPA_SHELL: &str = include_str!("../../csr/index.html");

/// Resolve the site path, then measure the frontend artifacts the embedded SPA
/// shell boots (raw, gzip, brotli). The targets are read from the shell (see
/// [`shell_boot_artifacts`]), so this Errs — naming the offending path — if the
/// build didn't emit a file the shell references.
pub fn run(site_path: Option<&str>) -> Result<AuditReport> {
    let site_path = resolve_site_path(site_path)?;
    let names = shell_boot_artifacts(SPA_SHELL)?;
    let mut artifacts = Vec::new();
    for name in &names {
        let path = Path::new(&site_path).join(name);
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "reading artifact {} (referenced by the SPA shell)",
                path.display()
            )
        })?;
        artifacts.push(ArtifactMetrics {
            path: path.to_string_lossy().into_owned(),
            raw_bytes: bytes.len() as u64,
            gzip_bytes: gzip_size(&bytes),
            brotli_bytes: brotli_size(&bytes),
        });
    }
    Ok(AuditReport {
        site_path,
        artifacts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_matches_script_rounding() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        // >= 10 in the unit → 0 decimals (the realistic MiB-range bundle path too)
        assert_eq!(format_bytes(10 * 1024), "10 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10 MiB");
        assert_eq!(format_bytes(1024_u64.pow(3)), "1.0 GiB");
    }

    #[test]
    fn compression_shrinks_repetitive_input_and_is_deterministic() {
        let bytes = vec![b'a'; 10_000];
        let g = gzip_size(&bytes);
        let b = brotli_size(&bytes);
        assert!(g < bytes.len() as u64, "gzip should shrink: {g}");
        assert!(b < bytes.len() as u64, "brotli should shrink: {b}");
        assert_eq!(g, gzip_size(&bytes), "gzip deterministic");
        assert_eq!(b, brotli_size(&bytes), "brotli deterministic");
    }

    #[test]
    fn render_table_has_header_site_path_and_relative_names() {
        // Two artifacts, as a real run emits — locks both the relative-naming and
        // the per-row column alignment that is the tool's whole output.
        let report = AuditReport {
            site_path: "/nix/store/x-jaunder-site".into(),
            artifacts: vec![
                ArtifactMetrics {
                    path: "/nix/store/x-jaunder-site/pkg/jaunder.wasm".into(),
                    raw_bytes: 2 * 1024 * 1024,
                    gzip_bytes: 700 * 1024,
                    brotli_bytes: 600 * 1024,
                },
                ArtifactMetrics {
                    path: "/nix/store/x-jaunder-site/pkg/jaunder.js".into(),
                    raw_bytes: 40 * 1024,
                    gzip_bytes: 12 * 1024,
                    brotli_bytes: 10 * 1024,
                },
            ],
        };
        let t = render_table(&report);
        assert!(t.contains("WASM bundle audit"));
        assert!(t.contains("site output: /nix/store/x-jaunder-site"));
        assert!(t.contains("artifact"));
        // relative paths, not the absolute ones
        assert!(t.contains("pkg/jaunder.wasm"));
        assert!(t.contains("pkg/jaunder.js"));
        assert!(!t.contains("/nix/store/x-jaunder-site/pkg/jaunder.wasm"));
        // each artifact renders its own row
        assert_eq!(t.lines().filter(|l| l.contains("pkg/")).count(), 2);
        // right-aligned size column, e.g. "2.0 MiB" for the wasm raw size
        assert!(t.contains("2.0 MiB"));
    }

    #[test]
    fn shell_boot_artifacts_reads_the_boot_urls_from_index_html() {
        // wasm first (the boot's `init(...)` target), then the JS module.
        let index = r#"<!doctype html><script type="module">
          import init from "/pkg/jaunder.js";
          init("/pkg/jaunder.wasm");
        </script>"#;
        assert_eq!(
            shell_boot_artifacts(index).unwrap(),
            vec!["pkg/jaunder.wasm".to_string(), "pkg/jaunder.js".to_string()]
        );
    }

    #[test]
    fn shell_boot_artifacts_rejects_arg_less_init() {
        // The exact #234 regression: arg-less init() carries no explicit wasm URL,
        // so wasm-bindgen falls back to its `_bg` default → the audit must refuse it.
        let index = r#"<script type="module">import init from "/pkg/jaunder.js"; init();</script>"#;
        let err = shell_boot_artifacts(index).unwrap_err().to_string();
        assert!(
            err.contains("234"),
            "error explains the #234 regression: {err}"
        );
    }

    #[test]
    fn run_errors_when_a_referenced_artifact_is_missing() {
        // The embedded SPA shell boots `/pkg/jaunder.wasm`; a site without that file
        // → run() must Err naming the missing artifact, without ever invoking `nix`.
        let dir = std::env::temp_dir().join(format!("audit-wasm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let res = run(Some(dir.to_str().unwrap()));
        std::fs::remove_dir_all(&dir).ok();
        let err = res.unwrap_err().to_string();
        assert!(
            err.contains("jaunder.wasm"),
            "error names the missing artifact: {err}"
        );
    }
}

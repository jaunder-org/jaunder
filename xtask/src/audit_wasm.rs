//! `cargo xtask audit-wasm` — measure frontend bundle download weight.
//!
//! Host-side analysis tool (ADR-0028): it shells out to `nix build .#csrBundle`
//! and sizes the manifest-selected identity artifacts. Pure size/format helpers
//! are unit-tested; the `nix`/filesystem I/O lives in `run`/`resolve_site_path`.

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

/// Per-section and per-crate attribution of a wasm artifact (#836).
///
/// Deliberately a separate report from [`AuditReport`], because it describes a
/// **different artifact**: attribution needs the name section, and the shipped
/// bundle has none once `wasm-opt` has run. `total_bytes` here is therefore not
/// the download weight, and the rendered output says so.
#[derive(Debug, Serialize)]
pub struct BreakdownReport {
    pub artifact: String,
    pub total_bytes: u64,
    pub sections: Vec<crate::wasm_sections::SectionSize>,
    /// The code section's span — the denominator for the per-crate percentages.
    pub code_bytes: u64,
    pub crates: Vec<crate::wasm_symbols::CrateBytes>,
}

/// Human-readable byte size: whole numbers for bytes and for any value ≥ 10 in
/// its unit, one decimal otherwise.
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
    use flate2::{Compression, write::GzEncoder};
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

/// The human size table: a header, the bundle root, then one right-aligned
/// raw/gzip/brotli row per manifest-selected identity artifact.
pub fn render_table(report: &AuditReport) -> String {
    let mut s = String::new();
    s.push_str("WASM bundle audit\n");
    s.push_str(&format!("bundle output: {}\n", report.site_path));
    s.push_str("artifact          raw        gzip       brotli\n");
    for row in &report.artifacts {
        let name = Path::new(&row.path)
            .strip_prefix(&report.site_path)
            .map(|path| path.to_string_lossy().into_owned())
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

/// Resolve the `.#csrBundle` output to audit. Returns `explicit` verbatim when
/// set (audit a prebuilt bundle root, e.g. in CI or while iterating); otherwise
/// runs the deterministic `nix build .#csrBundle` and parses its store path
/// (shared [`nix_build::build_out_path`]).
pub fn resolve_site_path(explicit: Option<&str>) -> Result<String> {
    match explicit {
        Some(p) => Ok(p.to_string()),
        None => nix_build::build_out_path("csrBundle"),
    }
}

/// Verify that the rendered shell selects exactly the runtime URLs declared by
/// the manifest. The shell intentionally starts fetching WASM before importing
/// the glue, but it must not preload WASM: #866 measured no boot benefit.
fn verify_generated_shell(index_html: &str, glue_path: &str, wasm_path: &str) -> Result<()> {
    let glue_url = format!("/{glue_path}");
    let wasm_url = format!("/{wasm_path}");
    anyhow::ensure!(
        index_html.matches(&glue_url).count() == 1
            && index_html.contains(&format!("import {{initMeasured}} from \"{glue_url}\";")),
        "generated index.html does not select manifest glue URL {glue_url:?}"
    );
    anyhow::ensure!(
        !index_html.contains(&format!("href=\"{wasm_url}\"")),
        "generated index.html preloads manifest wasm URL {wasm_url:?}; #866 requires no WASM preload"
    );
    anyhow::ensure!(
        index_html.matches(&wasm_url).count() == 1
            && index_html.contains(&format!("const __jaunderWasmUrl = \"{wasm_url}\";"))
            && index_html.contains("window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);")
            && index_html.contains("initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl);"),
        "generated index.html does not select manifest wasm URL {wasm_url:?}"
    );
    Ok(())
}

/// The manifest-selected identity artifacts the rendered CSR shell boots.
///
/// The manifest is the only parser, verifier, and name owner. Verifying its
/// exact inventory before returning paths makes a missing or inconsistent
/// selected artifact fail before any size measurement can hide the drift.
fn bundle_boot_artifacts(root: &Path) -> Result<Vec<String>> {
    let manifest_path = root.join("manifest.json");
    let manifest = csr_bundle::Manifest::from_json(
        &std::fs::read(&manifest_path)
            .with_context(|| format!("reading CSR bundle manifest {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parsing CSR bundle manifest {}", manifest_path.display()))?;
    manifest
        .verify_bundle(root)
        .with_context(|| format!("verifying CSR bundle {}", root.display()))?;

    let glue_path = manifest.role(csr_bundle::Role::Glue)?.path.clone();
    let wasm_path = manifest.role(csr_bundle::Role::Wasm)?.path.clone();
    let index_path = root.join("index.html");
    let index_html = std::fs::read_to_string(&index_path)
        .with_context(|| format!("reading generated CSR shell {}", index_path.display()))?;
    verify_generated_shell(&index_html, &glue_path, &wasm_path)?;
    Ok(vec![wasm_path, glue_path])
}

/// Resolve the site path, verify its build-only manifest and generated shell,
/// then measure the manifest-selected identity artifacts (raw, gzip, brotli).
pub fn run(site_path: Option<&str>) -> Result<AuditReport> {
    let site_path = resolve_site_path(site_path)?;
    let names = bundle_boot_artifacts(Path::new(&site_path))?;
    let mut artifacts = Vec::new();
    for name in &names {
        let path = Path::new(&site_path).join(name);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading manifest-selected artifact {}", path.display()))?;
        // Guard the strip (#836). `wasm-opt` drops the name section unless `-g` is
        // passed, so its reappearance means the optimisation pass was weakened or
        // lost — a 1.28 MiB regression that is otherwise invisible until someone
        // reads the size table and wonders.
        if name.ends_with(".wasm") && has_name_section(&bytes)? {
            anyhow::bail!(
                "{} carries a wasm name section: `wasm-opt` should have stripped it. \
                 Did the optimisation pass get `-g`, or get dropped? (#836)",
                path.display()
            );
        }
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

/// Resolve the wasm to attribute: `explicit` verbatim when set, otherwise the
/// `lib/csr.wasm` inside a fresh `nix build .#csrWasm`.
///
/// `.#csrWasm` rather than `.#site` on purpose — it is the pre-wasm-bindgen,
/// unstripped artifact, so it still carries the name section that attribution
/// reads. The shipped bundle cannot answer this question at all.
fn resolve_breakdown_path(explicit: Option<&str>) -> Result<String> {
    match explicit {
        Some(p) => Ok(p.to_string()),
        None => Ok(Path::new(&nix_build::build_out_path("csrWasm")?)
            .join("lib/csr.wasm")
            .to_string_lossy()
            .into_owned()),
    }
}

/// Attribute a wasm artifact's bytes to sections and crates.
pub fn breakdown(wasm_path: Option<&str>) -> Result<BreakdownReport> {
    let artifact = resolve_breakdown_path(wasm_path)?;
    let bytes =
        std::fs::read(&artifact).with_context(|| format!("reading wasm artifact {artifact}"))?;
    let sections = crate::wasm_sections::section_sizes(&bytes)
        .with_context(|| format!("parsing sections of {artifact}"))?;
    let code_bytes = sections
        .iter()
        .find(|s| s.name == "code")
        .map(|s| s.bytes)
        .unwrap_or(0);
    let crates = crate::wasm_symbols::rollup(&crate::wasm_symbols::function_sizes(&bytes)?);
    Ok(BreakdownReport {
        artifact,
        total_bytes: bytes.len() as u64,
        sections,
        code_bytes,
        crates,
    })
}

/// The breakdown tables: sections denominated on the file, then crates
/// denominated on the code section — each denominator named where it is used, so
/// a percentage cannot be read against the wrong whole.
pub fn render_breakdown(report: &BreakdownReport) -> String {
    let mut s = String::new();
    s.push_str("WASM bundle breakdown\n");
    s.push_str(&format!("artifact: {}\n", report.artifact));
    s.push_str(&format!(
        "total: {} — this is the unstripped pre-wasm-bindgen artifact, \
         NOT the shipped bundle size (see `cargo xtask audit-wasm` for that)\n",
        format_bytes(report.total_bytes)
    ));
    s.push('\n');

    s.push_str("section               bytes     share of file\n");
    for sec in &report.sections {
        s.push_str(&format!(
            "{:<18}  {:>9}  {:>12}\n",
            sec.name,
            format_bytes(sec.bytes),
            percent(sec.bytes, report.total_bytes),
        ));
    }

    s.push('\n');
    s.push_str(&format!(
        "crate attribution, share of the code section ({})\n",
        format_bytes(report.code_bytes)
    ));
    s.push_str("crate                 bytes     share of code section\n");
    for c in &report.crates {
        s.push_str(&format!(
            "{:<18}  {:>9}  {:>12}\n",
            c.krate,
            format_bytes(c.bytes),
            percent(c.bytes, report.code_bytes),
        ));
    }
    s
}

/// Whether a wasm carries a `name` custom section.
///
/// The shipped bundle must not: at 1.28 MiB it is the single largest thing
/// `wasm-opt` removes (#836). Attribution reads names from `.#csrWasm` instead,
/// so nothing needs them here.
pub fn has_name_section(wasm: &[u8]) -> Result<bool> {
    Ok(crate::wasm_sections::section_sizes(wasm)?
        .iter()
        .any(|s| s.name == "custom:name"))
}

/// `part` as a percentage of `whole`, to one decimal. A zero denominator renders
/// as `n/a` rather than a division result.
fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "n/a".to_string();
    }
    format!("{:.1}%", (part as f64 / whole as f64) * 100.0)
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
    fn render_table_has_header_bundle_root_and_relative_names() {
        let report = AuditReport {
            site_path: "/nix/store/x-jaunder-site".into(),
            artifacts: vec![
                ArtifactMetrics {
                    path: "/nix/store/x-jaunder-site/pkg/wasm-digest.wasm".into(),
                    raw_bytes: 2 * 1024 * 1024,
                    gzip_bytes: 700 * 1024,
                    brotli_bytes: 600 * 1024,
                },
                ArtifactMetrics {
                    path: "/nix/store/x-jaunder-site/pkg/glue-digest.js".into(),
                    raw_bytes: 40 * 1024,
                    gzip_bytes: 12 * 1024,
                    brotli_bytes: 10 * 1024,
                },
            ],
        };
        let table = render_table(&report);
        assert!(table.contains("WASM bundle audit"));
        assert!(table.contains("bundle output: /nix/store/x-jaunder-site"));
        assert!(table.contains("artifact"));
        assert!(table.contains("pkg/wasm-digest.wasm"));
        assert!(table.contains("pkg/glue-digest.js"));
        assert!(!table.contains("/nix/store/x-jaunder-site/pkg/wasm-digest.wasm"));
        assert_eq!(
            table.lines().filter(|line| line.contains("pkg/")).count(),
            2
        );
        assert!(table.contains("2.0 MiB"));
    }

    fn fixture_asset(
        root: &Path,
        role: csr_bundle::Role,
        extension: &str,
        identity: &[u8],
    ) -> csr_bundle::Asset {
        let path = format!("pkg/{}.{}", csr_bundle::digest(identity), extension);
        let gzip = format!("{extension}-gzip").into_bytes();
        let brotli = format!("{extension}-brotli").into_bytes();
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::write(root.join(&path), identity).unwrap();
        std::fs::write(root.join(format!("{path}.gz")), &gzip).unwrap();
        std::fs::write(root.join(format!("{path}.br")), &brotli).unwrap();
        csr_bundle::Asset {
            role: Some(role),
            path: path.clone(),
            sha256: csr_bundle::digest(identity),
            representations: std::collections::BTreeMap::from([
                (
                    "identity".into(),
                    csr_bundle::Representation {
                        path: path.clone(),
                        sha256: csr_bundle::digest(identity),
                    },
                ),
                (
                    "gzip".into(),
                    csr_bundle::Representation {
                        path: format!("{path}.gz"),
                        sha256: csr_bundle::digest(&gzip),
                    },
                ),
                (
                    "br".into(),
                    csr_bundle::Representation {
                        path: format!("{path}.br"),
                        sha256: csr_bundle::digest(&brotli),
                    },
                ),
            ]),
        }
    }

    fn fixture_bundle(root: &Path, shell: impl FnOnce(&str, &str) -> String) -> (String, String) {
        let wasm = fixture_asset(root, csr_bundle::Role::Wasm, "wasm", b"\0asm\x01\0\0\0");
        let glue = fixture_asset(root, csr_bundle::Role::Glue, "js", b"export {}");
        let manifest = csr_bundle::Manifest {
            version: csr_bundle::VERSION,
            assets: vec![glue.clone(), wasm.clone()],
        };
        std::fs::write(root.join("manifest.json"), manifest.to_json().unwrap()).unwrap();
        std::fs::write(
            root.join("index.html"),
            shell(&format!("/{}", glue.path), &format!("/{}", wasm.path)),
        )
        .unwrap();
        (wasm.path, glue.path)
    }

    fn generated_shell(glue_url: &str, wasm_url: &str) -> String {
        format!(
            "<script>const __jaunderWasmUrl = \"{wasm_url}\"; \
             window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);</script>\
             <script type=\"module\">import {{initMeasured}} from \"{glue_url}\"; \
             initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl);</script>"
        )
    }

    #[test]
    fn bundle_boot_artifacts_selects_manifest_roles() {
        let root = tempfile::tempdir().unwrap();
        let (wasm, glue) = fixture_bundle(root.path(), generated_shell);
        assert_eq!(
            bundle_boot_artifacts(root.path()).unwrap(),
            vec![wasm, glue]
        );
    }

    #[test]
    fn bundle_boot_artifacts_rejects_shell_manifest_inconsistency() {
        let root = tempfile::tempdir().unwrap();
        fixture_bundle(root.path(), |glue_url, _wasm_url| {
            generated_shell(glue_url, "/pkg/not-the-manifest-wasm.wasm")
        });
        let error = bundle_boot_artifacts(root.path()).unwrap_err().to_string();
        assert!(error.contains("manifest wasm URL"), "{error}");
    }

    #[test]
    fn bundle_boot_artifacts_rejects_wasm_preload() {
        let root = tempfile::tempdir().unwrap();
        fixture_bundle(root.path(), |glue_url, wasm_url| {
            format!(
                "{}<link rel=\"preload\" href=\"{wasm_url}\">",
                generated_shell(glue_url, wasm_url)
            )
        });
        let error = bundle_boot_artifacts(root.path()).unwrap_err().to_string();
        assert!(error.contains("preloads manifest wasm"), "{error}");
    }

    fn breakdown_fixture() -> BreakdownReport {
        use crate::wasm_sections::SectionSize;
        use crate::wasm_symbols::{CrateBytes, UNATTRIBUTED};
        BreakdownReport {
            artifact: "/nix/store/x-csr-wasm/lib/csr.wasm".into(),
            total_bytes: 5_350_591,
            sections: vec![
                SectionSize {
                    name: "code".into(),
                    bytes: 4_000_000,
                },
                SectionSize {
                    name: "data".into(),
                    bytes: 1_000_000,
                },
                SectionSize {
                    name: "custom:name".into(),
                    bytes: 350_591,
                },
            ],
            code_bytes: 4_000_000,
            crates: vec![
                CrateBytes {
                    krate: "orgize".into(),
                    bytes: 2_000_000,
                },
                CrateBytes {
                    krate: UNATTRIBUTED.into(),
                    bytes: 1_500_000,
                },
                CrateBytes {
                    krate: "core".into(),
                    bytes: 500_000,
                },
            ],
        }
    }

    #[test]
    fn render_breakdown_names_the_artifact_and_disclaims_shipped_size() {
        let t = render_breakdown(&breakdown_fixture());
        assert!(t.contains("/nix/store/x-csr-wasm/lib/csr.wasm"), "{t}");
        assert!(
            t.to_lowercase().contains("not the shipped"),
            "must state its total is not the shipped bundle size: {t}"
        );
    }

    #[test]
    fn render_breakdown_states_percentages_against_a_named_denominator() {
        let t = render_breakdown(&breakdown_fixture());
        // orgize is 2 MiB of the 4 MiB code section => 50%, denominated on the
        // code section, NOT on the 5.1 MiB file.
        assert!(t.contains("50.0%"), "{t}");
        assert!(
            t.contains("code section"),
            "the denominator must be named in the output: {t}"
        );
    }

    #[test]
    fn render_breakdown_shows_every_section_and_the_unattributed_bucket() {
        let t = render_breakdown(&breakdown_fixture());
        for s in ["code", "data", "custom:name"] {
            assert!(t.contains(s), "missing section {s}: {t}");
        }
        assert!(t.contains(crate::wasm_symbols::UNATTRIBUTED), "{t}");
    }

    #[test]
    fn detects_a_present_name_section() {
        let wasm = crate::wasm_symbols::tests_support::named_module();
        assert!(has_name_section(&wasm).unwrap());
    }

    #[test]
    fn detects_an_absent_name_section() {
        let wasm = crate::wasm_symbols::tests_support::unnamed_module();
        assert!(!has_name_section(&wasm).unwrap());
    }

    #[test]
    fn percent_of_a_zero_denominator_is_not_a_division() {
        assert_eq!(percent(0, 0), "n/a");
        assert_eq!(percent(1, 4), "25.0%");
    }

    #[test]
    fn breakdown_errors_when_the_artifact_is_missing() {
        let missing = "/nonexistent/csr.wasm";
        let err = breakdown(Some(missing)).unwrap_err().to_string();
        assert!(err.contains("csr.wasm"), "error names the artifact: {err}");
    }

    #[test]
    fn run_errors_when_a_manifest_selected_artifact_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let (wasm, _) = fixture_bundle(root.path(), generated_shell);
        std::fs::remove_file(root.path().join(&wasm)).unwrap();

        let error = run(Some(root.path().to_str().unwrap())).unwrap_err();
        let chain = format!("{error:#}");
        assert!(
            chain.contains(&wasm) && chain.contains("missing bundle file"),
            "error names the manifest-selected artifact and cause: {chain}"
        );
    }
}

//! `devtool csr-bundle` — post-process a built `csr.wasm` into the served CSR
//! bundle. Runs `wasm-bindgen --target web` over the wasm, then renames the
//! wasm-bindgen output (`csr.js` / `csr_bg.wasm`) to the `jaunder` names the
//! embedded SPA shell imports (`/pkg/jaunder.js`, `/pkg/jaunder.wasm`) and
//! rewrites the JS glue's internal wasm reference to match.
//!
//! This is the single implementation of the bundle post-processing, shared by
//! the host build (`cargo xtask build-csr`) and the Nix `csrWasmBundle`
//! derivation (#236) — replacing the inline `wasm-bindgen` + `mv` + `sed` the
//! flake ran, so host and Nix can no longer drift. Byte-identical to those
//! steps. Wasm-only: the served CSS is committed + rust-embedded
//! (`server/assets/`), not part of this bundle.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use flate2::write::GzEncoder;
use flate2::Compression;

/// wasm-bindgen names its outputs after the input stem; our input is `csr.wasm`.
const IN_JS: &str = "csr.js";
const IN_WASM: &str = "csr_bg.wasm";
/// The output name the SPA shell (`csr/index.html`) imports.
const OUT_JS: &str = "jaunder.js";
const OUT_WASM: &str = "jaunder.wasm";

/// Rewrite the wasm-bindgen JS glue's reference to its wasm file from the
/// `csr_bg.wasm` default to the renamed `jaunder.wasm`. Matches the flake's
/// `sed 's/csr_bg\.wasm/jaunder.wasm/g'` (literal, all occurrences). Pure —
/// only the `.wasm` filename is rewritten, not bare `csr_bg` identifiers.
fn rewrite_wasm_ref(js: &str) -> String {
    js.replace(IN_WASM, OUT_WASM)
}

/// Brotli-compress `bytes` at max quality (11, lgwin 22) — the release-asset
/// setting; the bundle is compressed once at build time, not per request.
fn brotli_compress(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut w = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
        w.write_all(bytes).context("brotli write")?;
    }
    Ok(out)
}

/// Gzip-compress `bytes` at best compression.
fn gzip_compress(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut e = GzEncoder::new(Vec::new(), Compression::best());
    e.write_all(bytes).context("gzip write")?;
    e.finish().context("gzip finish")
}

/// Append `.<ext>` to a path (e.g. `jaunder.wasm` -> `jaunder.wasm.br`).
fn with_suffix(path: &Path, ext: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

/// Write brotli (`.br`) and gzip (`.gz`) precompressed siblings next to `path`,
/// so the server can serve a precompressed variant by content negotiation
/// without compressing per request (#237). Only the top-level JS/wasm are
/// precompressed; wasm-bindgen `snippets/` are tiny and served identity.
fn write_precompressed(path: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let br = with_suffix(path, "br");
    std::fs::write(&br, brotli_compress(&bytes)?)
        .with_context(|| format!("writing {}", br.display()))?;
    let gz = with_suffix(path, "gz");
    std::fs::write(&gz, gzip_compress(&bytes)?)
        .with_context(|| format!("writing {}", gz.display()))?;
    Ok(())
}

/// Run `wasm-bindgen --target web` over `wasm` into `out`, then rename the
/// outputs to the `jaunder` names, fix the JS wasm reference, and write
/// precompressed (`.br`/`.gz`) siblings for the JS/wasm. Byte-identical to the
/// flake's inline `csrWasmBundle` steps for the raw outputs.
pub fn run(wasm: &Path, out: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating out dir {}", out.display()))?;

    let status = Command::new("wasm-bindgen")
        .arg("--target")
        .arg("web")
        .arg("--out-dir")
        .arg(out)
        .arg(wasm)
        .status()
        .context("spawning wasm-bindgen (is it on PATH?)")?;
    if !status.success() {
        bail!("wasm-bindgen failed ({status}) for {}", wasm.display());
    }

    std::fs::rename(out.join(IN_JS), out.join(OUT_JS))
        .with_context(|| format!("renaming {IN_JS} -> {OUT_JS} in {}", out.display()))?;
    std::fs::rename(out.join(IN_WASM), out.join(OUT_WASM))
        .with_context(|| format!("renaming {IN_WASM} -> {OUT_WASM} in {}", out.display()))?;

    let js_path = out.join(OUT_JS);
    let js = std::fs::read_to_string(&js_path)
        .with_context(|| format!("reading {}", js_path.display()))?;
    std::fs::write(&js_path, rewrite_wasm_ref(&js))
        .with_context(|| format!("writing {}", js_path.display()))?;

    // Precompress the final JS (post wasm-ref rewrite) and the wasm.
    write_precompressed(&js_path).context("precompressing jaunder.js")?;
    write_precompressed(&out.join(OUT_WASM)).context("precompressing jaunder.wasm")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_wasm_reference_but_not_bare_identifier() {
        let js = r#"const p = new URL('csr_bg.wasm', import.meta.url); export {csr_bg};"#;
        assert_eq!(
            rewrite_wasm_ref(js),
            r#"const p = new URL('jaunder.wasm', import.meta.url); export {csr_bg};"#,
        );
    }

    #[test]
    fn rewrites_all_occurrences() {
        let js = "a='csr_bg.wasm'; b='csr_bg.wasm';";
        assert_eq!(rewrite_wasm_ref(js), "a='jaunder.wasm'; b='jaunder.wasm';");
    }

    #[test]
    fn already_renamed_is_unchanged() {
        let js = "init('jaunder.wasm')";
        assert_eq!(rewrite_wasm_ref(js), js);
    }

    #[test]
    fn brotli_round_trips_and_shrinks() {
        use std::io::Read;
        let input = b"the quick brown fox jumps over the lazy dog".repeat(50);
        let compressed = brotli_compress(&input).unwrap();
        assert!(
            compressed.len() < input.len(),
            "brotli should shrink repetitive input"
        );
        let mut decoded = Vec::new();
        brotli::Decompressor::new(compressed.as_slice(), 4096)
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn gzip_round_trips_and_shrinks() {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let input = b"the quick brown fox jumps over the lazy dog".repeat(50);
        let compressed = gzip_compress(&input).unwrap();
        assert!(
            compressed.len() < input.len(),
            "gzip should shrink repetitive input"
        );
        let mut decoded = Vec::new();
        GzDecoder::new(compressed.as_slice())
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn with_suffix_appends_dotted_ext() {
        assert_eq!(
            with_suffix(Path::new("a/jaunder.wasm"), "br"),
            PathBuf::from("a/jaunder.wasm.br")
        );
    }
}

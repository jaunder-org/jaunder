//! `devtool csr-bundle` — post-process a built `csr.wasm` into the served CSR
//! bundle. Runs `wasm-bindgen --target web` over the wasm, then renames the
//! wasm-bindgen output (`csr.js` / `csr_bg.wasm`) to the `jaunder` names the
//! embedded SPA shell imports (`/pkg/jaunder.js`, `/pkg/jaunder.wasm`) and
//! rewrites the JS glue's internal wasm reference to match.
//!
//! This is the single implementation of the bundle post-processing, shared by
//! the host build (`cargo xtask build-csr`) and the Nix `csrWasmBundle`
//! derivation (#236) — so host and Nix cannot drift. Wasm-only: the served CSS
//! is committed + rust-embedded (`server/assets/`), not part of this bundle.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use flate2::Compression;
use flate2::write::GzEncoder;

/// wasm-bindgen names its outputs after the input stem; our input is `csr.wasm`.
const IN_JS: &str = "csr.js";
const IN_WASM: &str = "csr_bg.wasm";
/// The output name the SPA shell (`csr/index.html`) imports.
const OUT_JS: &str = "jaunder.js";
const OUT_WASM: &str = "jaunder.wasm";
const EXPERIMENT_SHAPE_SECTION_NAME: &str = "jaunder.shape";

/// The `wasm-opt` optimisation level, pinned by measurement on the real bundle
/// (#836) — raw bytes of the shipped `pkg/jaunder.wasm`:
///
/// | level        | raw bytes | vs none |
/// | ------------ | --------- | ------- |
/// | none         | 5 350 591 | —       |
/// | `-O2`        | 2 390 164 | −55.3%  |
/// | `-Os`        | 2 357 119 | −55.9%  |
/// | **`-Oz`**    | **2 267 063** | **−57.6%** |
///
/// Size is the objective, not speed: firefox spends ~88 ms compiling each MiB of
/// this file (#818), while the Rust-side mount path it produces measures
/// 1.7–12.7 ms. A slower-but-smaller artifact is the right trade here.
const WASM_OPT_LEVEL: &str = "-Oz";

/// Target features `rustc` enables by default for `wasm32-unknown-unknown`,
/// paired with the flag binaryen knows them by: `(rustc cfg, binaryen flag)`.
///
/// **The two vocabularies are not the same**, which is the reason this is a table
/// of pairs rather than one list. `rustc --print cfg` reports
/// `nontrapping-fptoint`; binaryen calls that feature
/// `nontrapping-float-to-int` and rejects the rustc spelling outright. The
/// mismatch surfaces as a hard build failure, not as silent under-optimisation,
/// so keeping the rustc name alongside is what lets the next person re-derive
/// this list from `rustc --print cfg` without rediscovering the divergence.
///
/// Listed explicitly rather than passing `-all`, which would track whatever the
/// installed binaryen considers "all" and let an upgrade change the accepted
/// input set with no diff to review.
const WASM_TARGET_FEATURES: [(&str, &str); 6] = [
    ("bulk-memory", "bulk-memory"),
    ("multivalue", "multivalue"),
    ("mutable-globals", "mutable-globals"),
    ("nontrapping-fptoint", "nontrapping-float-to-int"),
    ("reference-types", "reference-types"),
    ("sign-ext", "sign-ext"),
];

/// Rewrite the wasm-bindgen JS glue's reference to its wasm file from the
/// `csr_bg.wasm` default to the renamed `jaunder.wasm`. Matches the flake's
/// `sed 's/csr_bg\.wasm/jaunder.wasm/g'` (literal, all occurrences). Pure —
/// only the `.wasm` filename is rewritten, not bare `csr_bg` identifiers.
fn rewrite_wasm_ref(js: &str) -> String {
    js.replace(IN_WASM, OUT_WASM)
}

/// Refuse generated glue that can no longer consume a pre-started
/// `Promise<Response>` through wasm-bindgen's existing streaming/buffered path.
///
/// The early-fetch shell contract depends on these generated semantics. Pinning
/// them here turns a wasm-bindgen upgrade into a build failure instead of a
/// silent second request or a response-body delivery change.
fn ensure_promise_response_input_contract(js: &str) -> anyhow::Result<()> {
    for (behavior, fragment) in [
        (
            "recognize a resolved Response",
            "typeof Response === 'function' && module instanceof Response",
        ),
        (
            "stream the resolved Response",
            "WebAssembly.instantiateStreaming(module, imports)",
        ),
        (
            "buffer only the streaming fallback",
            "const bytes = await module.arrayBuffer()",
        ),
        (
            "instantiate buffered fallback bytes",
            "WebAssembly.instantiate(bytes, imports)",
        ),
        (
            "fetch only string, Request, or URL initializer inputs",
            "if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL))",
        ),
        (
            "start the URL-like input fetch",
            "module_or_path = fetch(module_or_path)",
        ),
        (
            "await a Promise<Response> before delivery",
            "__wbg_load(await module_or_path, imports)",
        ),
    ] {
        if !js.contains(fragment) {
            bail!("wasm-bindgen glue no longer {behavior}; missing {fragment:?}");
        }
    }
    Ok(())
}

/// Append the Jaunder-owned initializer without changing wasm-bindgen's default
/// export. The wrapper surrounds the real delivery path, so its timings retain
/// streaming versus buffered fallback behavior instead of introducing a second
/// byte-first path.
fn append_measured_initializer(js: &str, experiment_arm: Option<&str>) -> String {
    let experiment_arm = serde_json::to_string(&experiment_arm).expect("serializes string option");
    format!(
        "{js}\n\
\n\
const __jaunderWasmExperimentArm = {experiment_arm};\n\
const __jaunderWasmModuleShape = (module) => {{\n\
    if (!(module instanceof WebAssembly.Module)) {{\n\
        return null;\n\
    }}\n\
    const imports = WebAssembly.Module.imports(module);\n\
    const exports = WebAssembly.Module.exports(module);\n\
    const countKind = (items, kind) => items.filter((item) => item.kind === kind).length;\n\
    return {{\n\
        imports: imports.length,\n\
        importedFunctions: countKind(imports, \"function\"),\n\
        importedTables: countKind(imports, \"table\"),\n\
        importedMemories: countKind(imports, \"memory\"),\n\
        exports: exports.length,\n\
        exportedFunctions: countKind(exports, \"function\"),\n\
        exportedTables: countKind(exports, \"table\"),\n\
        exportedMemories: countKind(exports, \"memory\"),\n\
        customSections: WebAssembly.Module.customSections(module, \"jaunder.shape\").length,\n\
    }};\n\
}};\n\
\n\
export async function initMeasured(moduleOrPath) {{\n\
    performance.mark(\"jaunder.wasm.init_start\");\n\
    let path = null;\n\
    let apiMs = null;\n\
    let moduleShape = null;\n\
    const originalStreaming = WebAssembly.instantiateStreaming;\n\
    const originalInstantiate = WebAssembly.instantiate;\n\
    const measure = (original, successfulPath) => async function (...args) {{\n\
        const startedAt = performance.now();\n\
        const result = await original.apply(this, args);\n\
        path = successfulPath;\n\
        apiMs = performance.now() - startedAt;\n\
        moduleShape = __jaunderWasmModuleShape(result?.module ?? (result instanceof WebAssembly.Module ? result : null));\n\
        return result;\n\
    }};\n\
    if (typeof originalStreaming === \"function\") {{\n\
        WebAssembly.instantiateStreaming = measure(originalStreaming, \"streaming\");\n\
    }}\n\
    if (typeof originalInstantiate === \"function\") {{\n\
        WebAssembly.instantiate = measure(originalInstantiate, \"buffered\");\n\
    }}\n\
    try {{\n\
        const exports = await __wbg_init(moduleOrPath);\n\
        if (path !== null && apiMs !== null) {{\n\
            performance.mark(\"jaunder.wasm.init_done\", {{ detail: {{ path, apiMs, experimentArm: __jaunderWasmExperimentArm, moduleShape }} }});\n\
        }}\n\
        return exports;\n\
    }} finally {{\n\
        WebAssembly.instantiateStreaming = originalStreaming;\n\
        WebAssembly.instantiate = originalInstantiate;\n\
    }}\n\
}}\n"
    )
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

fn encode_u32_leb(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn custom_section(name: &str, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    encode_u32_leb(name.len() as u32, &mut payload);
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(data);

    let mut section = vec![0];
    encode_u32_leb(payload.len() as u32, &mut section);
    section.extend_from_slice(&payload);
    section
}

fn append_shape_sections(wasm: &Path, label: &str, count: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        count > 0,
        "--wasm-shape-section-count must be greater than zero"
    );
    let mut bytes = std::fs::read(wasm).with_context(|| format!("reading {}", wasm.display()))?;
    anyhow::ensure!(
        bytes.starts_with(b"\0asm\x01\0\0\0"),
        "{} is not a wasm module",
        wasm.display()
    );
    for index in 0..count {
        let payload = format!("{label}:{index}");
        bytes.extend_from_slice(&custom_section(
            EXPERIMENT_SHAPE_SECTION_NAME,
            payload.as_bytes(),
        ));
    }
    std::fs::write(wasm, bytes).with_context(|| format!("writing {}", wasm.display()))
}

/// The `wasm-opt` argument vector: optimisation level, an explicit enable for
/// every target feature rustc emits, then input and output.
///
/// Note there is no `-g`: the wasm name section is ~3.3 MiB of the unstripped
/// artifact and pure weight in production, so it is deliberately discarded.
/// Attribution reads it from `.#csrWasm` instead — see `cargo xtask audit-wasm
/// --breakdown`.
fn wasm_opt_args(level: &str, input: &Path, output: &Path) -> Vec<String> {
    let mut args = vec![level.to_string()];
    for (_rustc_cfg, binaryen_flag) in WASM_TARGET_FEATURES {
        args.push(format!("--enable-{binaryen_flag}"));
    }
    args.push(input.to_string_lossy().into_owned());
    args.push("-o".to_string());
    args.push(output.to_string_lossy().into_owned());
    args
}

/// Run `wasm-opt` over `wasm` in place, via a sibling temp file.
///
/// In-place is what the caller wants, but binaryen reads and writes streaming, so
/// naming the same path for both would truncate the input out from under it.
fn run_wasm_opt(wasm: &Path) -> anyhow::Result<()> {
    let tmp = with_suffix(wasm, "opt");
    let status = Command::new("wasm-opt")
        .args(wasm_opt_args(WASM_OPT_LEVEL, wasm, &tmp))
        .status()
        .context("spawning wasm-opt (is it on PATH?)")?;
    if !status.success() {
        bail!("wasm-opt failed ({status}) for {}", wasm.display());
    }
    std::fs::rename(&tmp, wasm)
        .with_context(|| format!("replacing {} with the optimised wasm", wasm.display()))?;
    Ok(())
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
pub fn run(
    wasm: &Path,
    out: &Path,
    experiment_arm: Option<&str>,
    shape_section: Option<&str>,
    shape_section_count: u32,
) -> anyhow::Result<()> {
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
    ensure_promise_response_input_contract(&js)
        .context("checking wasm-bindgen Promise<Response> initializer contract")?;
    std::fs::write(
        &js_path,
        append_measured_initializer(&rewrite_wasm_ref(&js), experiment_arm),
    )
    .with_context(|| format!("writing {}", js_path.display()))?;

    // Optimise before compressing, so the `.br`/`.gz` siblings describe the bytes
    // that actually ship.
    run_wasm_opt(&out.join(OUT_WASM)).context("optimising jaunder.wasm")?;
    if let Some(label) = shape_section {
        append_shape_sections(&out.join(OUT_WASM), label, shape_section_count)
            .context("adding wasm shape section")?;
    }

    // Precompress the final JS (post wasm-ref rewrite) and the wasm.
    write_precompressed(&js_path).context("precompressing jaunder.js")?;
    write_precompressed(&out.join(OUT_WASM)).context("precompressing jaunder.wasm")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WASM_BINDGEN_PROMISE_RESPONSE_GLUE: &str = r#"
async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            return await WebAssembly.instantiateStreaming(module, imports);
        }
        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    }
}
async function __wbg_init(module_or_path) {
    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }
    const { instance, module } = await __wbg_load(await module_or_path, imports);
}
"#;

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
    fn accepts_wasm_bindgen_promise_response_delivery_contract() {
        assert!(ensure_promise_response_input_contract(WASM_BINDGEN_PROMISE_RESPONSE_GLUE).is_ok());
    }

    #[test]
    fn rejects_wasm_bindgen_glue_that_stops_awaiting_the_input_promise() {
        let drifted = WASM_BINDGEN_PROMISE_RESPONSE_GLUE.replace(
            "__wbg_load(await module_or_path, imports)",
            "__wbg_load(module_or_path, imports)",
        );

        let error = ensure_promise_response_input_contract(&drifted).unwrap_err();
        assert!(
            error.to_string().contains("await a Promise<Response>"),
            "{error:#}"
        );
    }

    #[test]
    fn appends_measured_initializer_after_renaming_wasm_reference() {
        let js = append_measured_initializer("export { initSync, __wbg_init as default };", None);
        assert!(
            js.starts_with("export { initSync, __wbg_init as default };"),
            "{js}"
        );
        for expected in [
            "export async function initMeasured(moduleOrPath)",
            "const __jaunderWasmExperimentArm = null;",
            "const __jaunderWasmModuleShape = (module)",
            "WebAssembly.Module.imports(module)",
            "WebAssembly.Module.exports(module)",
            "performance.mark(\"jaunder.wasm.init_start\")",
            "performance.mark(\"jaunder.wasm.init_done\", { detail: { path, apiMs, experimentArm: __jaunderWasmExperimentArm, moduleShape } })",
            "WebAssembly.Module.customSections(module, \"jaunder.shape\")",
            "WebAssembly.instantiateStreaming",
            "WebAssembly.instantiate",
            "performance.now()",
            "WebAssembly.instantiateStreaming = originalStreaming",
            "WebAssembly.instantiate = originalInstantiate",
            "finally",
        ] {
            assert!(js.contains(expected), "missing {expected:?} in {js}");
        }
        assert!(
            js.find("export { initSync, __wbg_init as default };")
                .unwrap()
                < js.find("export async function initMeasured").unwrap(),
            "{js}"
        );
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
    fn wasm_opt_args_carry_the_pinned_level() {
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        assert!(args.contains(&WASM_OPT_LEVEL.to_string()), "{args:?}");
    }

    #[test]
    fn wasm_opt_args_enable_every_rustc_target_feature() {
        // Binaryen rejects input using features it was not told to allow, so an
        // unflagged run can hard-fail the build (#836). The list mirrors
        // `rustc --print cfg --target wasm32-unknown-unknown`.
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        for (_rustc_cfg, binaryen_flag) in WASM_TARGET_FEATURES {
            assert!(
                args.contains(&format!("--enable-{binaryen_flag}")),
                "missing --enable-{binaryen_flag} in {args:?}"
            );
        }
    }

    #[test]
    fn passes_binaryen_spellings_not_rustc_cfg_names() {
        // The two vocabularies diverge, and binaryen rejects the rustc spelling
        // with "Unknown option" — a hard build failure, found the hard way (#836).
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        assert!(
            args.contains(&"--enable-nontrapping-float-to-int".to_string()),
            "{args:?}"
        );
        assert!(
            !args.contains(&"--enable-nontrapping-fptoint".to_string()),
            "the rustc cfg spelling is not a binaryen flag: {args:?}"
        );
    }

    #[test]
    fn every_rustc_cfg_name_is_covered_exactly_once() {
        // The table is meant to be re-derivable from `rustc --print cfg`; a
        // duplicated or missing row would silently drop a feature.
        let mut cfgs: Vec<&str> = WASM_TARGET_FEATURES.iter().map(|(c, _)| *c).collect();
        cfgs.sort_unstable();
        let mut unique = cfgs.clone();
        unique.dedup();
        assert_eq!(cfgs, unique, "duplicate rustc cfg in the table");
        assert_eq!(
            cfgs,
            [
                "bulk-memory",
                "multivalue",
                "mutable-globals",
                "nontrapping-fptoint",
                "reference-types",
                "sign-ext",
            ]
        );
    }

    #[test]
    fn custom_section_encodes_name_and_payload() {
        assert_eq!(custom_section("x", b"y"), vec![0, 3, 1, b'x', b'y']);
    }

    #[test]
    fn shape_section_appends_to_wasm_module() {
        let dir = tempfile::tempdir().unwrap();
        let wasm = dir.path().join("module.wasm");
        std::fs::write(&wasm, b"\0asm\x01\0\0\0").unwrap();
        append_shape_sections(&wasm, "arm-shape", 2).unwrap();
        let bytes = std::fs::read(&wasm).unwrap();
        assert!(bytes.starts_with(b"\0asm\x01\0\0\0"), "{bytes:?}");
        assert!(bytes.ends_with(&custom_section(
            EXPERIMENT_SHAPE_SECTION_NAME,
            b"arm-shape:1",
        )));
        assert_eq!(
            bytes
                .windows(custom_section(EXPERIMENT_SHAPE_SECTION_NAME, b"arm-shape:0").len())
                .filter(|window| *window
                    == custom_section(EXPERIMENT_SHAPE_SECTION_NAME, b"arm-shape:0"))
                .count(),
            1
        );
    }

    #[test]
    fn wasm_opt_args_never_use_all_features() {
        // `-all` silently tracks whatever the installed binaryen considers "all",
        // so a binaryen upgrade could change the accepted input set with no diff.
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        assert!(
            !args.iter().any(|a| a == "-all" || a == "--all-features"),
            "{args:?}"
        );
    }

    #[test]
    fn wasm_opt_args_name_input_and_output() {
        let args = wasm_opt_args(
            WASM_OPT_LEVEL,
            Path::new("a/in.wasm"),
            Path::new("b/out.wasm"),
        );
        assert!(args.contains(&"a/in.wasm".to_string()), "{args:?}");
        let o = args.iter().position(|a| a == "-o").expect("has -o");
        assert_eq!(args[o + 1], "b/out.wasm", "{args:?}");
    }

    #[test]
    fn wasm_opt_does_not_request_debug_names() {
        // `-g` would retain the name section; the shipped bundle must not (#836).
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("in.wasm"), Path::new("out.wasm"));
        assert!(!args.iter().any(|a| a == "-g"), "{args:?}");
    }

    #[test]
    fn wasm_opt_writes_to_a_distinct_path_from_its_input() {
        // Streaming in and out of the same file would truncate the input.
        let args = wasm_opt_args(WASM_OPT_LEVEL, Path::new("x.wasm"), Path::new("x.wasm.opt"));
        let o = args.iter().position(|a| a == "-o").expect("has -o");
        assert_ne!(args[o + 1], "x.wasm");
    }

    #[test]
    fn with_suffix_appends_dotted_ext() {
        assert_eq!(
            with_suffix(Path::new("a/jaunder.wasm"), "br"),
            PathBuf::from("a/jaunder.wasm.br")
        );
    }
}

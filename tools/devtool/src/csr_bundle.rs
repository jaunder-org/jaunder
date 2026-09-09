//! Produces the build-only manifest and content-addressed `/pkg` CSR runtime bundle.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use csr_bundle::{Asset, Manifest, Representation, Role};
use flate2::Compression;
use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, ImportExpression, Statement};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;
use serde::Serialize;

const IN_JS: &str = "csr.js";
const IN_WASM: &str = "csr_bg.wasm";
const EXPERIMENT_SHAPE_SECTION_NAME: &str = "jaunder.shape";

/// The `wasm-opt` optimisation level, pinned by measurement after the custom
/// Theme Package management surface landed (#836, #1052, #1272, #1341):
///
/// | level        | raw bytes |
/// | ------------ | --------- |
/// | `-O2`        | 3 522 092 |
/// | `-Os`        | 3 474 240 |
/// | **`-Oz`**    | **3 335 708** |
///
/// Size is the objective, not speed: firefox spends ~88 ms compiling each MiB of
/// this file (#818), while the Rust-side mount path it produces measures
/// 1.7–12.7 ms. A slower-but-smaller artifact is the right trade here.
const WASM_OPT_LEVEL: &str = "-Oz";
const WASM_TARGET_FEATURES: [(&str, &str); 6] = [
    ("bulk-memory", "bulk-memory"),
    ("multivalue", "multivalue"),
    ("mutable-globals", "mutable-globals"),
    ("nontrapping-fptoint", "nontrapping-float-to-int"),
    ("reference-types", "reference-types"),
    ("sign-ext", "sign-ext"),
];
const SHELL: &str = include_str!("../../../csr/index.html");

fn ensure_promise_response_input_contract(js: &str) -> anyhow::Result<()> {
    for fragment in [
        "typeof Response === 'function' && module instanceof Response",
        "WebAssembly.instantiateStreaming(module, imports)",
        "const bytes = await module.arrayBuffer()",
        "WebAssembly.instantiate(bytes, imports)",
        "__wbg_load(await module_or_path, imports)",
    ] {
        if !js.contains(fragment) {
            bail!(
                "wasm-bindgen glue no longer supports the early-fetch Promise<Response> contract: missing {fragment:?}"
            );
        }
    }
    if js.matches("fetch(module_or_path)").count() != 1 {
        bail!("wasm-bindgen glue must fetch URL-like initializer input exactly once");
    }
    Ok(())
}

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

fn brotli_compress(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
        writer.write_all(bytes).context("brotli write")?;
    }
    Ok(out)
}

/// `GzBuilder::mtime(0)` is essential: a wall-clock gzip header would make a
/// logically identical bundle receive a different representation digest.
fn gzip_compress(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    encoder.write_all(bytes).context("gzip write")?;
    encoder.finish().context("gzip finish")
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
    let mut bytes = fs::read(wasm)?;
    anyhow::ensure!(
        bytes.starts_with(b"\0asm\x01\0\0\0"),
        "{} is not a wasm module",
        wasm.display()
    );
    for index in 0..count {
        bytes.extend_from_slice(&custom_section(
            EXPERIMENT_SHAPE_SECTION_NAME,
            format!("{label}:{index}").as_bytes(),
        ));
    }
    fs::write(wasm, bytes)?;
    Ok(())
}
fn wasm_opt_args(level: &str, input: &Path, output: &Path) -> Vec<String> {
    let mut args = vec![level.into()];
    args.extend(
        WASM_TARGET_FEATURES
            .into_iter()
            .map(|(_, feature)| format!("--enable-{feature}")),
    );
    args.extend([
        input.to_string_lossy().into_owned(),
        "-o".into(),
        output.to_string_lossy().into_owned(),
    ]);
    args
}
fn run_wasm_opt(wasm_opt: &Path, wasm: &Path) -> anyhow::Result<()> {
    let temporary = wasm.with_extension("wasm.opt");
    let status = Command::new(wasm_opt)
        .args(wasm_opt_args(WASM_OPT_LEVEL, wasm, &temporary))
        .status()
        .with_context(|| format!("spawning {}", wasm_opt.display()))?;
    if !status.success() {
        bail!("wasm-opt failed ({status}) for {}", wasm.display());
    }
    fs::rename(temporary, wasm)?;
    Ok(())
}
fn path_for(bytes: &[u8], extension: &str) -> String {
    format!("pkg/{}.{}", csr_bundle::digest(bytes), extension)
}

fn representation(path: String, bytes: &[u8]) -> Representation {
    Representation {
        path,
        sha256: csr_bundle::digest(bytes),
    }
}

fn write_representations(
    root: &Path,
    path: &str,
    bytes: &[u8],
    compressed: bool,
) -> anyhow::Result<BTreeMap<String, Representation>> {
    fs::write(root.join(path), bytes)?;
    let mut representations =
        BTreeMap::from([("identity".into(), representation(path.into(), bytes))]);
    if compressed {
        let gzip = gzip_compress(bytes)?;
        let brotli = brotli_compress(bytes)?;
        for (encoding, suffix, compressed_bytes) in [("gzip", "gz", gzip), ("br", "br", brotli)] {
            let compressed_path = format!("{path}.{suffix}");
            fs::write(root.join(&compressed_path), &compressed_bytes)?;
            representations.insert(
                encoding.into(),
                representation(compressed_path, &compressed_bytes),
            );
        }
    }
    Ok(representations)
}

#[derive(Debug)]
struct ModuleSpecifier {
    start: usize,
    end: usize,
    value: String,
}

fn is_relative_js(value: &str) -> bool {
    (value.starts_with("./") || value.starts_with("../")) && value.ends_with(".js")
}

fn static_module_specifiers(source: &str, importer: &Path) -> anyhow::Result<Vec<ModuleSpecifier>> {
    struct DynamicRelativeImport {
        found: bool,
    }

    impl<'a> Visit<'a> for DynamicRelativeImport {
        fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
            if let Expression::StringLiteral(literal) = &expression.source
                && is_relative_js(literal.value.as_str())
            {
                self.found = true;
            }
            walk::walk_import_expression(self, expression);
        }
    }

    let allocator = Allocator::default();
    let ParserReturn {
        program,
        diagnostics,
        fatal_error,
        ..
    } = Parser::new(&allocator, source, SourceType::mjs()).parse();
    if fatal_error || !diagnostics.is_empty() {
        let diagnostics = diagnostics
            .iter()
            .map(|diagnostic| format!("{diagnostic:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "parsing runtime module {}: {diagnostics}",
            importer.display()
        );
    }
    let mut dynamic = DynamicRelativeImport { found: false };
    dynamic.visit_program(&program);
    anyhow::ensure!(
        !dynamic.found,
        "dynamic relative runtime import is unsupported in {}",
        importer.display()
    );
    Ok(program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ImportDeclaration(declaration) => Some(&declaration.source),
            Statement::ExportFromDeclaration(declaration) => Some(&declaration.source),
            Statement::ExportAllDeclaration(declaration) => Some(&declaration.source),
            _ => None,
        })
        .map(|literal| ModuleSpecifier {
            start: literal.span.start as usize,
            end: literal.span.end as usize,
            value: literal.value.as_str().to_owned(),
        })
        .collect())
}

/// Rewrite only statically declared module sources. The parser supplies byte
/// spans, so comments, data strings, escapes, templates, and non-ASCII bytes
/// outside the literal remain exactly as generated.
fn rewrite_js_imports(
    source: &str,
    importer: &Path,
    files: &HashMap<PathBuf, String>,
) -> anyhow::Result<String> {
    let mut replacements = static_module_specifiers(source, importer)?
        .into_iter()
        .filter(|specifier| is_relative_js(&specifier.value))
        .map(|specifier| {
            let target = normalize(
                importer
                    .parent()
                    .expect("file has parent")
                    .join(&specifier.value),
            );
            let replacement = files.get(&target).ok_or_else(|| {
                anyhow::anyhow!(
                    "unresolved runtime import {:?} from {}",
                    specifier.value,
                    importer.display()
                )
            })?;
            let quote = source[specifier.start..specifier.end]
                .chars()
                .next()
                .expect("Oxc string literal span is nonempty");
            Ok((
                specifier.start,
                specifier.end,
                format!(
                    "{quote}./{}{quote}",
                    replacement
                        .strip_prefix("pkg/")
                        .expect("hashed path lives in pkg")
                ),
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    replacements.sort_by_key(|(start, _, _)| *start);
    let mut output = source.to_owned();
    for (start, end, replacement) in replacements.into_iter().rev() {
        output.replace_range(start..end, &replacement);
    }
    Ok(output)
}

fn relative_js_imports(source: &str, importer: &Path) -> anyhow::Result<Vec<PathBuf>> {
    static_module_specifiers(source, importer).map(|specifiers| {
        specifiers
            .into_iter()
            .filter(|specifier| is_relative_js(&specifier.value))
            .map(|specifier| {
                normalize(
                    importer
                        .parent()
                        .expect("file has parent")
                        .join(specifier.value),
                )
            })
            .collect()
    })
}

fn require_acyclic_complete_graph(glue: &Path, sources: &[PathBuf]) -> anyhow::Result<()> {
    fn visit(
        path: &Path,
        sources: &BTreeSet<PathBuf>,
        visiting: &mut BTreeSet<PathBuf>,
        visited: &mut BTreeSet<PathBuf>,
    ) -> anyhow::Result<()> {
        if visited.contains(path) {
            return Ok(());
        }
        if !visiting.insert(path.to_owned()) {
            bail!("cyclic runtime import graph includes {}", path.display());
        }
        let source = fs::read_to_string(path)
            .with_context(|| format!("reading runtime module {}", path.display()))?;
        for dependency in relative_js_imports(&source, path)? {
            anyhow::ensure!(
                sources.contains(&dependency),
                "unresolved runtime import from {}",
                path.display()
            );
            visit(&dependency, sources, visiting, visited)?;
        }
        visiting.remove(path);
        visited.insert(path.to_owned());
        Ok(())
    }

    let sources = sources.iter().cloned().collect();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    visit(glue, &sources, &mut visiting, &mut visited)?;
    if visited.len() != sources.len() {
        bail!("wasm-bindgen output contains unreferenced runtime JS modules");
    }
    Ok(())
}

fn normalize(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn js_sources(directory: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            sources.extend(js_sources(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "js") {
            sources.push(normalize(path));
        }
    }
    sources.sort();
    Ok(sources)
}

fn render_shell(glue: &str, wasm: &str) -> anyhow::Result<Vec<u8>> {
    let rendered = SHELL
        .replace("{{GLUE_URL}}", glue)
        .replace("{{WASM_URL}}", wasm);
    anyhow::ensure!(
        rendered.matches(glue).count() == 1,
        "shell must contain exactly one glue URL"
    );
    anyhow::ensure!(
        rendered.matches(wasm).count() == 1,
        "shell must contain exactly one wasm URL"
    );
    Ok(rendered.into_bytes())
}

const COVERAGE_SECTIONS: [&str; 2] = ["__llvm_covfun", "__llvm_covmap"];

/// Diagnostic artifacts emitted only by the separately instrumented CSR build.
///
/// Normal callers pass `None`, preserving the production bundle byte path.
pub struct DiagnosticArtifacts<'a> {
    /// Where to record whether wasm-bindgen plus wasm-opt retained mapping data.
    pub metadata_status: &'a Path,
    /// Where to record the exact producer/analyzer tool identities.
    pub toolchain_identity: &'a Path,
    /// The minicov crate version selected by the instrumented Cargo feature.
    pub minicov_version: &'a str,
}

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

fn read_u32_leb(bytes: &[u8], index: &mut usize, subject: &str) -> anyhow::Result<u32> {
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

/// A checked view of one wasm section. The payload excludes its id and length.
struct WasmSection<'a> {
    id: u8,
    payload: &'a [u8],
}

/// Checked iterator over a version-1 wasm module's sections.
struct WasmSections<'a> {
    wasm: &'a [u8],
    index: usize,
}

impl<'a> Iterator for WasmSections<'a> {
    type Item = anyhow::Result<WasmSection<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.wasm.len() {
            return None;
        }
        let id = match self.wasm.get(self.index) {
            Some(id) => *id,
            None => return Some(Err(anyhow::anyhow!("truncated wasm section id"))),
        };
        self.index += 1;
        let length = match read_u32_leb(self.wasm, &mut self.index, "wasm section length") {
            Ok(length) => length,
            Err(error) => return Some(Err(error)),
        };
        let end = match self.index.checked_add(length as usize) {
            Some(end) => end,
            None => return Some(Err(anyhow::anyhow!("wasm section length overflows module"))),
        };
        let payload = match self.wasm.get(self.index..end) {
            Some(payload) => payload,
            None => return Some(Err(anyhow::anyhow!("wasm section extends past module"))),
        };
        self.index = end;
        Some(Ok(WasmSection { id, payload }))
    }
}

fn wasm_sections(wasm: &[u8]) -> anyhow::Result<WasmSections<'_>> {
    if !wasm.starts_with(b"\0asm\x01\0\0\0") {
        bail!("not a wasm version-1 module");
    }
    Ok(WasmSections { wasm, index: 8 })
}

fn custom_section_name(section: &[u8]) -> anyhow::Result<&[u8]> {
    let mut index = 0;
    let length = read_u32_leb(section, &mut index, "wasm custom-section name length")?;
    let end = index
        .checked_add(length as usize)
        .context("wasm custom-section name length overflows section")?;
    section
        .get(index..end)
        .context("truncated wasm custom-section name")
}

fn coverage_section_presence(wasm: &[u8]) -> anyhow::Result<[bool; 2]> {
    let mut found = [false; COVERAGE_SECTIONS.len()];
    for section in wasm_sections(wasm)? {
        let section = section?;
        if section.id != 0 {
            continue;
        }
        let name = custom_section_name(section.payload)?;
        for (position, required) in COVERAGE_SECTIONS.iter().enumerate() {
            found[position] |= name == required.as_bytes();
        }
    }
    Ok(found)
}

fn wasm_exports(wasm: &[u8]) -> anyhow::Result<Vec<String>> {
    let mut exports = Vec::new();
    for section in wasm_sections(wasm)? {
        let section = section?;
        if section.id != 7 {
            continue;
        }
        let mut index = 0;
        let count = read_u32_leb(section.payload, &mut index, "wasm export count")?;
        for _ in 0..count {
            let name_length = read_u32_leb(section.payload, &mut index, "wasm export name length")?;
            let name_end = index
                .checked_add(name_length as usize)
                .context("wasm export name length overflows section")?;
            let name = std::str::from_utf8(
                section
                    .payload
                    .get(index..name_end)
                    .context("truncated wasm export name")?,
            )
            .context("wasm export name is not UTF-8")?;
            exports.push(name.to_owned());
            index = name_end;
            let _kind = *section
                .payload
                .get(index)
                .context("truncated wasm export kind")?;
            index += 1;
            let _ = read_u32_leb(section.payload, &mut index, "wasm export index")?;
        }
        anyhow::ensure!(
            index == section.payload.len(),
            "trailing data in wasm export section"
        );
    }
    Ok(exports)
}

fn wasm_custom_section_present(wasm: &[u8], wanted: &str) -> anyhow::Result<bool> {
    for section in wasm_sections(wasm)? {
        let section = section?;
        if section.id == 0 && custom_section_name(section.payload)? == wanted.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn wasm_boundary(artifact: &'static str, wasm: &[u8]) -> anyhow::Result<WasmBoundary> {
    Ok(WasmBoundary {
        artifact,
        coverage_sections: coverage_section_presence(wasm)?,
        wasm_bindgen_metadata: wasm_custom_section_present(wasm, "__wasm_bindgen_unstable")?,
        exports: wasm_exports(wasm)?,
    })
}

fn coverage_metadata_status(
    input: &[u8],
    after_wasm_bindgen: &[u8],
    after_wasm_opt: &[u8],
) -> anyhow::Result<CoverageMetadataStatus> {
    let input = wasm_boundary("csr.wasm", input)?;
    let after_wasm_bindgen = wasm_boundary("wasm-bindgen.wasm", after_wasm_bindgen)?;
    let after_wasm_opt = wasm_boundary("wasm-opt.wasm", after_wasm_opt)?;
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

fn command_version(program: &str, args: &[&str]) -> anyhow::Result<String> {
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

fn write_diagnostic_artifacts(
    artifacts: &DiagnosticArtifacts<'_>,
    input_wasm: &Path,
    wasm_bindgen_wasm: &Path,
    bundled_wasm: &Path,
) -> anyhow::Result<()> {
    let metadata = coverage_metadata_status(
        &fs::read(input_wasm).with_context(|| format!("reading {}", input_wasm.display()))?,
        &fs::read(wasm_bindgen_wasm)
            .with_context(|| format!("reading {}", wasm_bindgen_wasm.display()))?,
        &fs::read(bundled_wasm).with_context(|| format!("reading {}", bundled_wasm.display()))?,
    )?;
    fs::write(
        artifacts.metadata_status,
        serde_json::to_vec_pretty(&metadata).context("serializing coverage metadata status")?,
    )
    .with_context(|| format!("writing {}", artifacts.metadata_status.display()))?;
    let identity = ToolchainIdentity {
        version: 1,
        rustc: command_version("rustc", &["-Vv"])?,
        clang: command_version("clang", &["--version"])?,
        llvm_profdata: command_version("llvm-profdata", &["--version"])?,
        llvm_cov: command_version("llvm-cov", &["--version"])?,
        wasm_bindgen: command_version("wasm-bindgen", &["--version"])?,
        wasm_opt: command_version("wasm-opt", &["--version"])?,
        minicov: artifacts.minicov_version,
    };
    fs::write(
        artifacts.toolchain_identity,
        serde_json::to_vec_pretty(&identity).context("serializing toolchain identity")?,
    )
    .with_context(|| format!("writing {}", artifacts.toolchain_identity.display()))
}

/// Generate a content-addressed CSR bundle into a sibling temporary directory,
/// validate its exact inventory, then rename the complete bundle root into
/// place. Existing output is rejected instead of being partially overwritten.
struct BundleTools<'a> {
    wasm_bindgen: &'a Path,
    wasm_opt: &'a Path,
}

pub fn run(
    wasm: &Path,
    out: &Path,
    experiment_arm: Option<&str>,
    shape_section: Option<&str>,
    shape_section_count: u32,
    diagnostic_artifacts: Option<&DiagnosticArtifacts<'_>>,
) -> anyhow::Result<()> {
    run_with_tools(
        wasm,
        out,
        experiment_arm,
        shape_section,
        shape_section_count,
        BundleTools {
            wasm_bindgen: Path::new("wasm-bindgen"),
            wasm_opt: Path::new("wasm-opt"),
        },
        diagnostic_artifacts,
    )
}

fn run_with_tools(
    wasm: &Path,
    out: &Path,
    experiment_arm: Option<&str>,
    shape_section: Option<&str>,
    shape_section_count: u32,
    tools: BundleTools<'_>,
    diagnostic_artifacts: Option<&DiagnosticArtifacts<'_>>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !out.exists(),
        "refusing to replace existing bundle root {}",
        out.display()
    );
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix("csr-bundle-")
        .tempdir_in(parent)?;
    let root = temporary.path();
    let generated = root.join("generated");
    fs::create_dir(&generated)?;
    let status = Command::new(tools.wasm_bindgen)
        .args(["--target", "web", "--out-dir"])
        .arg(&generated)
        .arg(wasm)
        .status()
        .with_context(|| format!("spawning {}", tools.wasm_bindgen.display()))?;
    if !status.success() {
        bail!("wasm-bindgen failed ({status}) for {}", wasm.display());
    }
    let wasm_source = generated.join(IN_WASM);
    let wasm_bindgen_snapshot = wasm_source.with_extension("wasm.bindgen");
    if diagnostic_artifacts.is_some() {
        fs::copy(&wasm_source, &wasm_bindgen_snapshot).with_context(|| {
            format!(
                "capturing post-wasm-bindgen diagnostic module {}",
                wasm_source.display()
            )
        })?;
    }
    run_wasm_opt(tools.wasm_opt, &wasm_source)?;
    if let Some(label) = shape_section {
        append_shape_sections(&wasm_source, label, shape_section_count)?;
    }
    let wasm_bytes = fs::read(&wasm_source)?;
    if let Some(artifacts) = diagnostic_artifacts {
        write_diagnostic_artifacts(artifacts, wasm, &wasm_bindgen_snapshot, &wasm_source)?;
        fs::remove_file(&wasm_bindgen_snapshot).with_context(|| {
            format!(
                "removing diagnostic boundary snapshot {}",
                wasm_bindgen_snapshot.display()
            )
        })?;
    }
    let wasm_path = path_for(&wasm_bytes, "wasm");
    let glue_source = generated.join(IN_JS);
    let glue = fs::read_to_string(&glue_source)?;
    ensure_promise_response_input_contract(&glue)?;
    fs::write(
        &glue_source,
        append_measured_initializer(
            &glue.replace(IN_WASM, wasm_path.strip_prefix("pkg/").expect("pkg path")),
            experiment_arm,
        ),
    )?;
    let sources = js_sources(&generated)?;
    require_acyclic_complete_graph(&normalize(glue_source.clone()), &sources)?;
    let mut paths = HashMap::new();
    for source in &sources {
        let bytes = fs::read(source)?;
        paths.insert(source.clone(), path_for(&bytes, "js"));
    }
    // Iteration reaches a stable dependency-first naming because a module's
    // rewritten bytes include its already-final dependency names. A cycle never
    // stabilizes and is rejected before any output is committed.
    for _ in 0..sources.len() + 1 {
        let old = paths.clone();
        for source in &sources {
            let rewritten = rewrite_js_imports(&fs::read_to_string(source)?, source, &old)?;
            paths.insert(source.clone(), path_for(rewritten.as_bytes(), "js"));
        }
        if paths == old {
            break;
        }
    }
    for source in &sources {
        let rewritten = rewrite_js_imports(&fs::read_to_string(source)?, source, &paths)?;
        anyhow::ensure!(
            path_for(rewritten.as_bytes(), "js") == paths[source],
            "cyclic runtime import graph includes {}",
            source.display()
        );
    }
    fs::create_dir(root.join("pkg"))?;
    let mut assets = Vec::new();
    for source in sources {
        let bytes =
            rewrite_js_imports(&fs::read_to_string(&source)?, &source, &paths)?.into_bytes();
        let path = paths.remove(&source).expect("source path assigned");
        let is_glue = source == normalize(glue_source.clone());
        let representations = write_representations(root, &path, &bytes, is_glue)?;
        assets.push(Asset {
            role: is_glue.then_some(Role::Glue),
            path,
            sha256: csr_bundle::digest(&bytes),
            representations,
        });
    }
    let wasm_representations = write_representations(root, &wasm_path, &wasm_bytes, true)?;
    assets.push(Asset {
        role: Some(Role::Wasm),
        path: wasm_path,
        sha256: csr_bundle::digest(&wasm_bytes),
        representations: wasm_representations,
    });
    let manifest = Manifest {
        version: csr_bundle::VERSION,
        assets,
    };
    manifest
        .verify_bundle(root)
        .context("verifying generated CSR bundle")?;
    fs::write(root.join("manifest.json"), manifest.to_json()?)?;
    fs::write(
        root.join("index.html"),
        render_shell(
            &format!("/{}", manifest.role(Role::Glue)?.path),
            &format!("/{}", manifest.role(Role::Wasm)?.path),
        )?,
    )?;
    fs::remove_dir_all(&generated)?;
    let published_temp = temporary.keep();
    if let Err(error) = fs::rename(&published_temp, out) {
        if let Err(cleanup) = fs::remove_dir_all(&published_temp) {
            bail!(
                "publishing bundle root {}: {error}; removing temporary bundle {}: {cleanup}",
                out.display(),
                published_temp.display()
            );
        }
        return Err(error).with_context(|| format!("publishing bundle root {}", out.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    fn fixture_tools() -> (PathBuf, PathBuf) {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        (fixtures.join("wasm-bindgen"), fixtures.join("wasm-opt"))
    }

    fn produce_fixture(directory: &Path, name: &str) -> PathBuf {
        let input = directory.join("input.wasm");
        fs::write(&input, b"\0asm\x01\0\0\0").unwrap();
        let (wasm_bindgen, wasm_opt) = fixture_tools();
        let output = directory.join(name);
        run_with_tools(
            &input,
            &output,
            None,
            None,
            0,
            BundleTools {
                wasm_bindgen: &wasm_bindgen,
                wasm_opt: &wasm_opt,
            },
            None,
        )
        .unwrap();
        output
    }

    fn fixture_manifest(root: &Path) -> Manifest {
        Manifest::from_json(&fs::read(root.join("manifest.json")).unwrap()).unwrap()
    }
    fn wasm_with_sections(sections: &[&str]) -> Vec<u8> {
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        for section in sections {
            wasm.extend(custom_section(section, b"coverage"));
        }
        wasm
    }

    #[test]
    fn diagnostic_metadata_reports_preserved_sections_at_each_bundle_boundary() {
        let wasm = wasm_with_sections(&COVERAGE_SECTIONS);
        let status = coverage_metadata_status(&wasm, &wasm, &wasm).expect("well-formed test wasm");
        assert_eq!(status.result, "preserved");
        assert_eq!(status.input.coverage_sections, [true, true]);
        assert_eq!(status.after_wasm_bindgen.coverage_sections, [true, true]);
        assert_eq!(status.after_wasm_opt.coverage_sections, [true, true]);
    }

    #[test]
    fn diagnostic_metadata_reports_the_boundary_that_loses_coverage() {
        let manual_link = wasm_with_sections(&COVERAGE_SECTIONS);
        let wasm_bindgen = wasm_with_sections(&COVERAGE_SECTIONS);
        let wasm_opt = wasm_with_sections(&[COVERAGE_SECTIONS[0]]);
        let status = coverage_metadata_status(&manual_link, &wasm_bindgen, &wasm_opt)
            .expect("well-formed test wasm");
        assert_eq!(status.result, "lost-during-bundling");
        assert_eq!(status.after_wasm_bindgen.coverage_sections, [true, true]);
        assert_eq!(status.after_wasm_opt.coverage_sections, [true, false]);
    }

    #[test]
    fn checked_wasm_section_iterator_rejects_truncated_and_overlong_sections() {
        let truncated_custom_section = [b"\0asm\x01\0\0\0".as_slice(), &[0, 1]].concat();
        assert!(coverage_section_presence(&truncated_custom_section).is_err());

        let truncated_export = [b"\0asm\x01\0\0\0".as_slice(), &[7, 1, 1]].concat();
        assert!(wasm_exports(&truncated_export).is_err());

        let overlong_length = [
            b"\0asm\x01\0\0\0".as_slice(),
            &[0, 0x80, 0x80, 0x80, 0x80, 0x80],
        ]
        .concat();
        assert!(wasm_custom_section_present(&overlong_length, "name").is_err());
    }
    #[test]
    fn gzip_is_deterministic() {
        assert_eq!(
            gzip_compress(b"same bytes").unwrap(),
            gzip_compress(b"same bytes").unwrap()
        );
    }

    #[test]
    fn producer_fixture_is_deterministic_and_rewrites_the_runtime_graph() {
        let directory = tempfile::tempdir().unwrap();
        let first = produce_fixture(directory.path(), "first");
        let second = produce_fixture(directory.path(), "second");

        assert_eq!(
            fs::read(first.join("manifest.json")).unwrap(),
            fs::read(second.join("manifest.json")).unwrap()
        );

        let manifest = fixture_manifest(&first);
        manifest.verify_bundle(&first).unwrap();
        for role in [Role::Glue, Role::Wasm] {
            assert_eq!(
                manifest.role(role).unwrap().representations.len(),
                3,
                "{role:?} has every required representation"
            );
        }

        let glue = manifest.role(Role::Glue).unwrap();
        let glue_source = fs::read_to_string(first.join(&glue.path)).unwrap();
        let imports = static_module_specifiers(&glue_source, Path::new(&glue.path)).unwrap();
        let dependency = manifest
            .assets
            .iter()
            .find(|asset| asset.role.is_none())
            .unwrap();
        assert_eq!(
            imports
                .into_iter()
                .map(|specifier| specifier.value)
                .collect::<Vec<_>>(),
            vec![format!(
                "./{}",
                dependency.path.strip_prefix("pkg/").unwrap()
            )]
        );
    }

    #[test]
    fn producer_fixture_rejects_duplicate_roles() {
        let directory = tempfile::tempdir().unwrap();
        let root = produce_fixture(directory.path(), "bundle");
        let mut manifest = fixture_manifest(&root);
        let duplicate = manifest.role(Role::Glue).unwrap().clone();
        manifest.assets.push(duplicate);

        assert!(matches!(
            manifest.verify_bundle(&root),
            Err(csr_bundle::Error::DuplicateRole(Role::Glue))
        ));
    }

    #[test]
    fn producer_fixture_rejects_on_disk_digest_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        let root = produce_fixture(directory.path(), "bundle");
        let manifest = fixture_manifest(&root);
        let glue_path = manifest.role(Role::Glue).unwrap().path.clone();
        fs::write(root.join(&glue_path), b"tampered bundle bytes").unwrap();

        assert!(matches!(
            manifest.verify_bundle(&root),
            Err(csr_bundle::Error::DigestMismatch { path, .. }) if path == glue_path
        ));
    }
    #[test]
    fn nested_imports_use_final_dependency_names() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("a.js");
        let dep = directory.path().join("deep/b.js");
        fs::create_dir(dep.parent().unwrap()).unwrap();
        fs::write(&source, "import './deep/b.js'").unwrap();

        fs::write(&dep, "export const b = 1").unwrap();
        let files = HashMap::from([(normalize(dep), "pkg/final.js".into())]);
        assert_eq!(
            rewrite_js_imports(&fs::read_to_string(&source).unwrap(), &source, &files).unwrap(),
            "import './final.js'"
        );
    }

    #[test]
    fn rewrites_static_import_and_reexport_sources_only() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("a.js");
        let dependency = directory.path().join("b.js");
        fs::write(&dependency, "export {}").unwrap();
        let files = HashMap::from([(normalize(dependency), "pkg/final.js".into())]);
        let input = "import './b.js'; export { x } from './b.js'; export * from './b.js';";
        assert_eq!(
            rewrite_js_imports(input, &source, &files).unwrap(),
            "import './final.js'; export { x } from './final.js'; export * from './final.js';"
        );
    }

    #[test]
    fn preserves_non_ascii_comments_data_and_escaped_non_import_strings() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("a.js");
        let dependency = directory.path().join("b.js");
        fs::write(&dependency, "export {}").unwrap();
        let files = HashMap::from([(normalize(dependency), "pkg/final.js".into())]);
        let input = "// './b.js'\nconst note = \"./b.js\"; const café = '✓'; import './b\\x2ejs';";
        assert_eq!(
            rewrite_js_imports(input, &source, &files).unwrap(),
            "// './b.js'\nconst note = \"./b.js\"; const café = '✓'; import './final.js';"
        );
    }

    #[test]
    fn rejects_dynamic_relative_imports() {
        let source = Path::new("a.js");
        let error = rewrite_js_imports("import('./b.js')", source, &HashMap::new()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("dynamic relative runtime import")
        );
    }

    #[test]
    fn rejects_cycles_and_unreferenced_modules() {
        let directory = tempfile::tempdir().unwrap();
        let glue = directory.path().join("csr.js");
        let other = directory.path().join("other.js");
        fs::write(&glue, "import './other.js'").unwrap();
        fs::write(&other, "import './csr.js'").unwrap();
        let error = require_acyclic_complete_graph(
            &normalize(glue.clone()),
            &[normalize(glue), normalize(other)],
        )
        .unwrap_err();
        assert!(error.to_string().contains("cyclic runtime import graph"));
    }

    #[test]
    fn rejects_unreferenced_generated_module() {
        let directory = tempfile::tempdir().unwrap();
        let glue = directory.path().join("csr.js");
        let extra = directory.path().join("extra.js");
        fs::write(&glue, "export const glue = true").unwrap();
        fs::write(&extra, "export const extra = true").unwrap();
        let error = require_acyclic_complete_graph(
            &normalize(glue.clone()),
            &[normalize(glue), normalize(extra)],
        )
        .unwrap_err();
        assert!(error.to_string().contains("unreferenced"));
    }
    #[test]
    fn shell_has_one_url_per_role_and_preserves_fetch_before_init() {
        let shell =
            String::from_utf8(render_shell("/pkg/glue.js", "/pkg/module.wasm").unwrap()).unwrap();
        assert_eq!(shell.matches("/pkg/glue.js").count(), 1);
        assert_eq!(shell.matches("/pkg/module.wasm").count(), 1);
        assert!(shell.find("__jaunderWasmFetch").unwrap() < shell.find("initMeasured").unwrap());
    }
    #[test]
    fn final_hash_observes_rewritten_bytes() {
        let path = path_for(b"import './dependency.js'", "js");
        assert!(path.contains(&csr_bundle::digest(b"import './dependency.js'")));
    }
}

//! Concrete Task 2 repository-census collectors.
//!
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_parser::ParserReturn;
use oxc_span::SourceType;
use syn::visit::Visit;

use super::model::{Candidate, CollectorMetadata};
use super::{
    CellReport, CellState, CollectorContext, CollectorSpec, EvidenceMethod, Language, SignalFamily,
};

const STRUCTURAL_VERSION: &str = "1";

/// Task 2 registrations for the shared Task 1 collector interface.
pub(crate) fn specs() -> Vec<CollectorSpec> {
    vec![
        spec(
            SignalFamily::DependencyStructure,
            Language::Rust,
            rust_dependencies,
        ),
        spec(
            SignalFamily::DependencyStructure,
            Language::TypeScript,
            typescript_dependencies,
        ),
        spec(
            SignalFamily::DependencyStructure,
            Language::Elisp,
            elisp_dependencies,
        ),
        spec(
            SignalFamily::ExportedSymbolReferences,
            Language::Rust,
            rust_semantic_symbols,
        ),
        spec(
            SignalFamily::ExportedSymbolReferences,
            Language::TypeScript,
            typescript_semantic_symbols,
        ),
        spec(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::Rust,
            rust_unreferenced_symbols,
        ),
        spec(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::TypeScript,
            typescript_unreferenced_symbols,
        ),
        spec(
            SignalFamily::ClonesAndRepeatedTestShapes,
            Language::Rust,
            rust_clones,
        ),
        spec(
            SignalFamily::ClonesAndRepeatedTestShapes,
            Language::TypeScript,
            typescript_clones,
        ),
        spec(
            SignalFamily::ClonesAndRepeatedTestShapes,
            Language::Elisp,
            elisp_clones,
        ),
        spec(
            SignalFamily::ConversionAndErrorMapping,
            Language::Rust,
            rust_conversions,
        ),
        spec(
            SignalFamily::ConversionAndErrorMapping,
            Language::TypeScript,
            typescript_conversions,
        ),
    ]
}

fn spec(
    signal: SignalFamily,
    language: Language,
    collect: fn(&CollectorContext) -> CellReport,
) -> CollectorSpec {
    CollectorSpec {
        signal,
        language,
        collect,
    }
}

fn structural(
    signal: SignalFamily,
    language: Language,
    limitation: &str,
    candidates: Vec<Candidate>,
) -> CellReport {
    CellReport {
        signal,
        language,
        collector: CollectorMetadata {
            identity: format!("census-{}-structural", language_name(language)),
            version: Some(STRUCTURAL_VERSION.into()),
            evidence_method: EvidenceMethod::Structural,
            limitation: limitation.into(),
        },
        state: if candidates.is_empty() {
            CellState::Clean
        } else {
            CellState::Candidates { candidates }
        },
    }
}

fn failed(signal: SignalFamily, language: Language, identity: &str, error: String) -> CellReport {
    CellReport {
        signal,
        language,
        collector: CollectorMetadata {
            identity: identity.into(),
            version: Some(STRUCTURAL_VERSION.into()),
            evidence_method: EvidenceMethod::Structural,
            limitation: "source could not be parsed structurally".into(),
        },
        state: CellState::Failed { error },
    }
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::Rust => "rust",
        Language::TypeScript => "typescript",
        Language::Elisp => "elisp",
        Language::Repository => "repository",
    }
}

fn files(context: &CollectorContext, language: Language) -> impl Iterator<Item = (&str, &str)> {
    context.snapshot.files.iter().filter_map(move |file| {
        let selected = match language {
            Language::Rust => file.path.ends_with(".rs"),
            Language::TypeScript => [".ts", ".tsx", ".js", ".jsx"]
                .iter()
                .any(|suffix| file.path.ends_with(suffix)),
            Language::Elisp => file.path.ends_with(".el"),
            Language::Repository => false,
        };
        selected.then_some((file.path.as_str(), file.content.as_str()))
    })
}

struct RustUses {
    paths: BTreeSet<String>,
}
impl<'ast> Visit<'ast> for RustUses {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let rendered = quote::ToTokens::to_token_stream(&item.tree).to_string();
        if let Some(root) = rendered
            .split("::")
            .next()
            .filter(|root| !matches!(*root, "crate" | "self" | "super"))
        {
            self.paths.insert(root.replace(' ', ""));
        }
        syn::visit::visit_item_use(self, item);
    }
}

fn rust_dependencies(context: &CollectorContext) -> CellReport {
    let mut uses = BTreeMap::<String, Vec<String>>::new();
    for (path, source) in files(context, Language::Rust) {
        let parsed = match syn::parse_file(source) {
            Ok(file) => file,
            Err(error) => {
                return failed(
                    SignalFamily::DependencyStructure,
                    Language::Rust,
                    "census-rust-structural",
                    format!("{path}: {error}"),
                );
            }
        };
        let mut visitor = RustUses {
            paths: BTreeSet::new(),
        };
        visitor.visit_file(&parsed);
        for dependency in visitor.paths {
            uses.entry(dependency).or_default().push(path.into());
        }
    }
    let candidates = uses
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(dependency, paths)| Candidate {
            identity: format!("rust-dependency:{dependency}"),
            summary: format!(
                "structural Rust import `{dependency}` used in {} files",
                paths.len()
            ),
            paths,
        })
        .collect();
    structural(
        SignalFamily::DependencyStructure,
        Language::Rust,
        "imports identify syntax-level dependencies, not resolved Cargo package edges",
        candidates,
    )
}

fn parse_typescript(path: &str, source: &str) -> Result<(), String> {
    let allocator = Allocator::default();
    let ParserReturn {
        diagnostics,
        panicked,
        ..
    } = Parser::new(
        &allocator,
        source,
        SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts()),
    )
    .parse();
    if panicked || !diagnostics.is_empty() {
        Err(diagnostics
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("; "))
    } else {
        Ok(())
    }
}

fn typescript_dependencies(context: &CollectorContext) -> CellReport {
    let mut imports = BTreeMap::<String, Vec<String>>::new();
    for (path, source) in files(context, Language::TypeScript) {
        if let Err(error) = parse_typescript(path, source) {
            return failed(
                SignalFamily::DependencyStructure,
                Language::TypeScript,
                "census-typescript-structural",
                format!("{path}: {error}"),
            );
        }
        for module in source.lines().filter_map(import_module) {
            imports.entry(module).or_default().push(path.into());
        }
    }
    let candidates = imports
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(module, paths)| Candidate {
            identity: format!("typescript-dependency:{module}"),
            summary: format!(
                "parsed TypeScript import `{module}` used in {} files",
                paths.len()
            ),
            paths,
        })
        .collect();
    structural(
        SignalFamily::DependencyStructure,
        Language::TypeScript,
        "imports are parsed before extraction; aliases and runtime resolution are not modeled",
        candidates,
    )
}

fn import_module(line: &str) -> Option<String> {
    let line = line.trim();
    if !line.starts_with("import ") && !line.starts_with("export ") {
        return None;
    }
    let quoted = line
        .split_once(" from ")
        .map(|(_, module)| module.trim())
        .unwrap_or(line.strip_prefix("import ")?.trim());
    let quote = quoted.chars().next()?;
    (quote == '\'' || quote == '"').then_some(())?;
    quoted[1..]
        .split(quote)
        .next()
        .filter(|module| !module.is_empty())
        .map(str::to_owned)
}

fn elisp_dependencies(context: &CollectorContext) -> CellReport {
    let mut dependencies = BTreeMap::<String, Vec<String>>::new();
    for (path, source) in files(context, Language::Elisp) {
        if !balanced_elisp(source) {
            return failed(
                SignalFamily::DependencyStructure,
                Language::Elisp,
                "census-elisp-structural",
                format!("{path}: unbalanced Elisp forms"),
            );
        }
        for dependency in source.lines().filter_map(elisp_require) {
            dependencies
                .entry(dependency)
                .or_default()
                .push(path.into());
        }
    }
    let candidates = dependencies
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(dependency, paths)| Candidate {
            identity: format!("elisp-dependency:{dependency}"),
            summary: format!("Elisp require `{dependency}` used in {} files", paths.len()),
            paths,
        })
        .collect();
    structural(
        SignalFamily::DependencyStructure,
        Language::Elisp,
        "only top-level require forms are modeled; dynamic feature loading is excluded",
        candidates,
    )
}
fn elisp_require(line: &str) -> Option<String> {
    let form = line.trim().strip_prefix("(require '")?;
    form.split(')')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn rust_clones(context: &CollectorContext) -> CellReport {
    clones(context, Language::Rust)
}
fn typescript_clones(context: &CollectorContext) -> CellReport {
    clones(context, Language::TypeScript)
}
fn elisp_clones(context: &CollectorContext) -> CellReport {
    clones(context, Language::Elisp)
}
fn clones(context: &CollectorContext, language: Language) -> CellReport {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for (path, source) in files(context, language) {
        let valid = match language {
            Language::Rust => syn::parse_file(source)
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Language::TypeScript => parse_typescript(path, source),
            Language::Elisp => balanced_elisp(source)
                .then_some(())
                .ok_or_else(|| "unbalanced Elisp forms".into()),
            Language::Repository => Ok(()),
        };
        if let Err(error) = valid {
            return failed(
                SignalFamily::ClonesAndRepeatedTestShapes,
                language,
                &format!("census-{}-structural", language_name(language)),
                format!("{path}: {error}"),
            );
        }
        for block in candidate_blocks(source, language) {
            groups
                .entry(normalize_shape(block))
                .or_default()
                .push(path.into());
        }
    }
    let candidates = groups
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(shape, paths)| Candidate {
            identity: format!(
                "{}-shape:{:x}",
                language_name(language),
                stable_hash(&shape)
            ),
            summary: format!(
                "repeated parsed {} block shape across {} files",
                language_name(language),
                paths.len()
            ),
            paths,
        })
        .collect();
    structural(
        SignalFamily::ClonesAndRepeatedTestShapes,
        language,
        "normalized parsed block shapes are review candidates; formatting and identifier spelling are discarded",
        candidates,
    )
}
fn candidate_blocks(source: &str, language: Language) -> Vec<&str> {
    let markers: &[&str] = match language {
        Language::Rust => &["fn "],
        Language::TypeScript => &["function ", "test("],
        Language::Elisp => &["(defun ", "(ert-deftest "],
        Language::Repository => &[],
    };
    markers
        .iter()
        .flat_map(|marker| source.match_indices(marker))
        .filter_map(|(start, _)| block_after(&source[start..], language))
        .collect()
}
fn block_after(source: &str, language: Language) -> Option<&str> {
    let (open, close) = match language {
        Language::Elisp => ('(', ')'),
        _ => ('{', '}'),
    };
    let first = source.find(open)?;
    let mut depth = 0i32;
    for (offset, character) in source[first..].char_indices() {
        if character == open {
            depth += 1;
        }
        if character == close {
            depth -= 1;
            if depth == 0 {
                return Some(&source[..first + offset + 1]);
            }
        }
    }
    None
}
fn normalize_shape(source: &str) -> String {
    let mut out = String::new();
    let mut token = String::new();
    for character in source.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            token.push(character);
        } else {
            if !token.is_empty() {
                out.push_str(
                    if token.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        "#"
                    } else {
                        "id"
                    },
                );
                token.clear();
            }
            if !character.is_whitespace() {
                out.push(character);
            }
        }
    }
    if !token.is_empty() {
        out.push_str("id");
    }
    out
}
fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn rust_conversions(context: &CollectorContext) -> CellReport {
    conversion_sequences(context, Language::Rust)
}
fn typescript_conversions(context: &CollectorContext) -> CellReport {
    conversion_sequences(context, Language::TypeScript)
}
fn conversion_sequences(context: &CollectorContext, language: Language) -> CellReport {
    let mut candidates = Vec::new();
    for (path, source) in files(context, language) {
        let parsed = match language {
            Language::Rust => syn::parse_file(source)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Language::TypeScript => parse_typescript(path, source),
            _ => Ok(()),
        };
        if let Err(error) = parsed {
            return failed(
                SignalFamily::ConversionAndErrorMapping,
                language,
                &format!("census-{}-structural", language_name(language)),
                format!("{path}: {error}"),
            );
        }
        let found = match language {
            Language::Rust => {
                source.contains(".map_err(")
                    && (source.contains(".into()")
                        || source.contains("?")
                        || source.contains("From<"))
            }
            Language::TypeScript => {
                source.contains("catch (") && (source.contains("throw ") || source.contains("as "))
            }
            _ => false,
        };
        if found {
            candidates.push(Candidate {
                identity: format!("{}-conversion-error:{}", language_name(language), path),
                summary: format!(
                    "parsed {} conversion and error-mapping sequence",
                    language_name(language)
                ),
                paths: vec![path.into()],
            });
        }
    }
    structural(
        SignalFamily::ConversionAndErrorMapping,
        language,
        "parsed source is searched for conservative conversion/error sequence forms; it does not infer runtime error equivalence",
        candidates,
    )
}

fn rust_semantic_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::ExportedSymbolReferences,
        Language::Rust,
        "rust-analyzer",
    )
}
fn typescript_semantic_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::ExportedSymbolReferences,
        Language::TypeScript,
        "typescript-language-server",
    )
}
fn rust_unreferenced_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::UnusedDependenciesAndSymbols,
        Language::Rust,
        "rust-analyzer",
    )
}
fn typescript_unreferenced_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::UnusedDependenciesAndSymbols,
        Language::TypeScript,
        "typescript-language-server",
    )
}

fn semantic_symbols(
    context: &CollectorContext,
    signal: SignalFamily,
    language: Language,
    analyzer: &str,
) -> CellReport {
    let version = match Command::new(analyzer).arg("--version").output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CellReport::unavailable(
                signal,
                language,
                format!("semantic analyzer `{analyzer}` is not installed"),
            );
        }
        Err(error) => {
            return semantic_failed(
                signal,
                language,
                analyzer,
                None,
                format!("cannot execute semantic analyzer: {error}"),
            );
        }
        Ok(output) if !output.status.success() => {
            return semantic_failed(
                signal,
                language,
                analyzer,
                None,
                format!(
                    "semantic analyzer version probe failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            );
        }
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    };
    let mut client = match LspClient::start(context, language, analyzer) {
        Ok(client) => client,
        Err(error) => return semantic_failed(signal, language, analyzer, Some(version), error),
    };
    let symbols = match client.request("workspace/symbol", serde_json::json!({ "query": "" })) {
        Ok(response) => match symbols_from_response(context, response) {
            Ok(symbols) => symbols,
            Err(error) => return semantic_failed(signal, language, analyzer, Some(version), error),
        },
        Err(error) => return semantic_failed(signal, language, analyzer, Some(version), error),
    };
    let mut candidates = Vec::new();
    let mut exported_symbols = 0;
    for symbol in symbols {
        let exported = match client.request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": symbol.uri },
                "position": { "line": symbol.line, "character": symbol.character }
            }),
        ) {
            Ok(response) => hover_reports_exported(language, response),
            Err(error) => return semantic_failed(signal, language, analyzer, Some(version), error),
        };
        if !exported {
            continue;
        }
        exported_symbols += 1;
        let references = match client.request(
            "textDocument/references",
            serde_json::json!({
                "textDocument": { "uri": symbol.uri },
                "position": { "line": symbol.line, "character": symbol.character },
                "context": { "includeDeclaration": false }
            }),
        ) {
            Ok(response) => match references_from_response(context, response) {
                Ok(references) => references,
                Err(error) => {
                    return semantic_failed(signal, language, analyzer, Some(version), error);
                }
            },
            Err(error) => return semantic_failed(signal, language, analyzer, Some(version), error),
        };
        let emit = match signal {
            SignalFamily::ExportedSymbolReferences => !references.is_empty(),
            SignalFamily::UnusedDependenciesAndSymbols => references.is_empty(),
            _ => false,
        };
        if emit {
            let mut paths = references;
            paths.push(symbol.path);
            candidates.push(Candidate {
                identity: format!("semantic-symbol:{}", symbol.name),
                summary: format!(
                    "LSP reference analysis for exported symbol `{}`",
                    symbol.name
                ),
                paths,
            });
        }
    }
    if exported_symbols == 0 {
        return CellReport::unavailable(
            signal,
            language,
            format!("semantic analyzer `{analyzer}` did not report export visibility"),
        );
    }
    CellReport {
        signal,
        language,
        collector: CollectorMetadata {
            identity: format!("census-{}-semantic-lsp", language_name(language)),
            version: Some(version),
            evidence_method: EvidenceMethod::Semantic,
            limitation: "workspace/symbol, hover visibility, and textDocument/references depend on analyzer index and project configuration".into(),
        },
        state: if candidates.is_empty() { CellState::Clean } else { CellState::Candidates { candidates } },
    }
}

fn analyzer_stdio_args(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => &[],
        Language::TypeScript => &["--stdio"],
        Language::Elisp | Language::Repository => &[],
    }
}

struct LspClient {
    child: std::process::Child,
    input: std::process::ChildStdin,
    responses: Receiver<Result<serde_json::Value, String>>,
    next_id: u64,
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl LspClient {
    fn start(
        context: &CollectorContext,
        language: Language,
        analyzer: &str,
    ) -> Result<Self, String> {
        let mut command = Command::new(analyzer);
        command
            .args(analyzer_stdio_args(language))
            .current_dir(&context.repo_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| format!("starting `{analyzer}` LSP server: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| format!("`{analyzer}` did not provide LSP stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| format!("`{analyzer}` did not provide LSP stdout"))?;
        let (sender, responses) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let result = read_lsp_message(&mut output);
                let done = result.is_err();
                if sender.send(result).is_err() || done {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            responses,
            next_id: 1,
        };
        let root = absolute_repo_root(context)?;
        let root_uri = url::Url::from_directory_path(&root)
            .map_err(|_| format!("cannot form LSP root URI for {}", root.display()))?
            .to_string();
        client.request("initialize", serde_json::json!({
            "processId": null,
            "rootUri": root_uri,
            "capabilities": { "workspace": { "symbol": {} }, "textDocument": { "references": {} } }
        }))?;
        client.notify("initialized", serde_json::json!({}))?;
        Ok(client)
    }

    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )?;
        loop {
            let message = self.read()?;
            if message.get("method").is_some() && message.get("id").is_some() {
                self.send(serde_json::json!({ "jsonrpc": "2.0", "id": message["id"].clone(), "result": null }))?;
                continue;
            }
            if message.get("id").and_then(serde_json::Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!("LSP `{method}` failed: {error}"));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| format!("LSP `{method}` response omitted result"));
        }
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<(), String> {
        self.send(serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, message: serde_json::Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(&message)
            .map_err(|error| format!("serializing LSP request: {error}"))?;
        write!(self.input, "Content-Length: {}\r\n\r\n", bytes.len())
            .map_err(|error| format!("writing LSP header: {error}"))?;
        self.input
            .write_all(&bytes)
            .map_err(|error| format!("writing LSP body: {error}"))?;
        self.input
            .flush()
            .map_err(|error| format!("flushing LSP request: {error}"))
    }

    fn read(&mut self) -> Result<serde_json::Value, String> {
        self.responses
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("waiting for LSP response: {error}"))?
    }
}

fn read_lsp_message(
    output: &mut BufReader<std::process::ChildStdout>,
) -> Result<serde_json::Value, String> {
    let mut length = None;
    loop {
        let mut line = String::new();
        let read = output
            .read_line(&mut line)
            .map_err(|error| format!("reading LSP header: {error}"))?;
        if read == 0 {
            return Err("LSP server closed stdout".into());
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid LSP content length: {error}"))?,
            );
        }
    }
    let length = length.ok_or_else(|| "LSP response omitted Content-Length".to_owned())?;
    let mut body = vec![0; length];
    output
        .read_exact(&mut body)
        .map_err(|error| format!("reading LSP response body: {error}"))?;
    serde_json::from_slice(&body).map_err(|error| format!("malformed LSP JSON response: {error}"))
}

struct SemanticSymbol {
    name: String,
    uri: String,
    line: u32,
    character: u32,
    path: String,
}

fn symbols_from_response(
    context: &CollectorContext,
    response: serde_json::Value,
) -> Result<Vec<SemanticSymbol>, String> {
    let entries = response
        .as_array()
        .ok_or_else(|| "LSP workspace/symbol result is not an array".to_owned())?;
    let mut symbols = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "LSP workspace/symbol entry omitted name".to_owned())?
            .to_owned();
        let location = entry
            .get("location")
            .ok_or_else(|| "LSP workspace/symbol entry omitted location".to_owned())?;
        let uri = location
            .get("uri")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "LSP workspace/symbol location omitted URI".to_owned())?
            .to_owned();
        let start = location
            .get("range")
            .and_then(|range| range.get("start"))
            .ok_or_else(|| "LSP workspace/symbol location omitted range start".to_owned())?;
        let line = start
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "LSP workspace/symbol range start omitted line".to_owned())?
            as u32;
        let character = start
            .get("character")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "LSP workspace/symbol range start omitted character".to_owned())?
            as u32;
        let path = uri_to_relative(context, &uri)
            .ok_or_else(|| format!("LSP symbol URI is outside repository: {uri}"))?;
        symbols.push(SemanticSymbol {
            name,
            uri,
            line,
            character,
            path,
        });
    }
    Ok(symbols)
}
fn hover_reports_exported(language: Language, response: serde_json::Value) -> bool {
    let text = response
        .get("contents")
        .and_then(|contents| {
            contents
                .get("value")
                .and_then(serde_json::Value::as_str)
                .or_else(|| contents.as_str())
        })
        .unwrap_or_default();
    match language {
        Language::Rust => text.contains("pub "),
        Language::TypeScript => text.contains("export "),
        Language::Elisp | Language::Repository => false,
    }
}

fn references_from_response(
    context: &CollectorContext,
    response: serde_json::Value,
) -> Result<Vec<String>, String> {
    let entries = response
        .as_array()
        .ok_or_else(|| "LSP textDocument/references result is not an array".to_owned())?;
    entries
        .iter()
        .map(|entry| {
            let uri = entry
                .get("uri")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "LSP reference entry omitted URI".to_owned())?;
            uri_to_relative(context, uri)
                .ok_or_else(|| format!("LSP reference URI is outside repository: {uri}"))
        })
        .collect()
}

fn semantic_failed(
    signal: SignalFamily,
    language: Language,
    analyzer: &str,
    version: Option<String>,
    error: String,
) -> CellReport {
    CellReport {
        signal,
        language,
        collector: CollectorMetadata {
            identity: format!("census-{}-semantic-lsp", language_name(language)),
            version,
            evidence_method: EvidenceMethod::Semantic,
            limitation: format!("requires `{analyzer}` LSP workspace symbols and references"),
        },
        state: CellState::Failed { error },
    }
}

fn absolute_repo_root(context: &CollectorContext) -> Result<std::path::PathBuf, String> {
    if context.repo_root.is_absolute() {
        Ok(context.repo_root.clone())
    } else {
        std::env::current_dir()
            .map(|current| current.join(&context.repo_root))
            .map_err(|error| format!("resolving census repository root: {error}"))
    }
}

fn uri_to_relative(context: &CollectorContext, uri: &str) -> Option<String> {
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    let root = absolute_repo_root(context).ok()?;
    Some(path.strip_prefix(root).ok()?.to_str()?.replace('\\', "/"))
}
fn balanced_elisp(source: &str) -> bool {
    source.chars().fold(0i32, |depth, character| {
        depth + i32::from(character == '(') - i32::from(character == ')')
    }) == 0
}

#[cfg(test)]
mod tests {
    use super::super::SourceSnapshot;
    use super::super::snapshot::SourceFile;
    use super::*;
    fn context(files: &[(&str, &str)]) -> CollectorContext {
        CollectorContext {
            repo_root: ".".into(),
            snapshot: SourceSnapshot {
                files: files
                    .iter()
                    .map(|(path, content)| SourceFile {
                        path: (*path).into(),
                        content: (*content).into(),
                    })
                    .collect(),
            },
        }
    }
    #[test]
    fn rust_dependency_positive_and_negative() {
        assert!(matches!(
            rust_dependencies(&context(&[
                ("a.rs", "use serde::Serialize;"),
                ("b.rs", "use serde::Deserialize;")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            rust_dependencies(&context(&[("a.rs", "use crate::x;")])).state,
            CellState::Clean
        ));
    }
    #[test]
    fn typescript_dependency_positive_and_negative() {
        assert!(matches!(
            typescript_dependencies(&context(&[
                ("a.ts", "import x from 'pkg';"),
                ("b.ts", "import y from 'pkg';")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            typescript_dependencies(&context(&[
                ("a.ts", "import x from 'a';"),
                ("b.ts", "import y from 'b';")
            ]))
            .state,
            CellState::Clean
        ));
    }
    #[test]
    fn elisp_dependency_positive_and_negative() {
        assert!(matches!(
            elisp_dependencies(&context(&[
                ("a.el", "(require 'dash)"),
                ("b.el", "(require 'dash)")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            elisp_dependencies(&context(&[("a.el", "(require 'dash)")])).state,
            CellState::Clean
        ));
    }
    #[test]
    fn semantic_protocol_transcripts_are_standard_lsp_shapes() {
        let context = CollectorContext {
            repo_root: "/repo".into(),
            snapshot: SourceSnapshot::default(),
        };
        let symbols = symbols_from_response(
            &context,
            serde_json::json!([{
                "name": "public_api",
                "kind": 12,
                "location": {
                    "uri": "file:///repo/src/lib.rs",
                    "range": { "start": { "line": 4, "character": 7 }, "end": { "line": 4, "character": 17 } }
                }
            }]),
        )
        .expect("standard workspace/symbol response parses");
        assert_eq!(symbols[0].path, "src/lib.rs");
        let references = references_from_response(
            &context,
            serde_json::json!([{
                "uri": "file:///repo/src/caller.rs",
                "range": { "start": { "line": 8, "character": 2 }, "end": { "line": 8, "character": 12 } }
            }]),
        )
        .expect("standard textDocument/references response parses");
        assert_eq!(references, ["src/caller.rs"]);
        assert!(symbols_from_response(&context, serde_json::json!({})).is_err());
        assert!(references_from_response(&context, serde_json::json!({})).is_err());
    }
    #[test]
    fn semantic_hover_filters_private_symbols() {
        assert!(hover_reports_exported(
            Language::Rust,
            serde_json::json!({ "contents": { "kind": "markdown", "value": "```rust\npub fn api()\n```" } })
        ));
        assert!(!hover_reports_exported(
            Language::Rust,
            serde_json::json!({ "contents": { "kind": "markdown", "value": "```rust\nfn private()\n```" } })
        ));
        assert!(hover_reports_exported(
            Language::TypeScript,
            serde_json::json!({ "contents": { "kind": "markdown", "value": "export function api(): void" } })
        ));
        assert!(!hover_reports_exported(
            Language::TypeScript,
            serde_json::json!({ "contents": "function privateOnly(): void" })
        ));
    }
    #[test]
    fn analyzer_stdio_launch_contract_is_language_specific() {
        assert!(analyzer_stdio_args(Language::Rust).is_empty());
        assert_eq!(analyzer_stdio_args(Language::TypeScript), ["--stdio"]);
    }
    #[test]
    fn missing_semantic_analyzer_is_unavailable() {
        let report = semantic_symbols(
            &context(&[]),
            SignalFamily::ExportedSymbolReferences,
            Language::Rust,
            "jaunder-census-definitely-missing-analyzer",
        );
        assert!(matches!(report.state, CellState::Unavailable { .. }));
    }
    #[test]
    fn clone_positive_and_negative() {
        assert!(matches!(
            rust_clones(&context(&[
                ("a.rs", "fn a(){let x=1; x+1;}"),
                ("b.rs", "fn b(){let y=2; y+2;}")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            rust_clones(&context(&[
                ("a.rs", "fn a(){}"),
                ("b.rs", "fn b(){let y=2;}")
            ]))
            .state,
            CellState::Clean
        ));
    }
    #[test]
    fn conversion_positive_and_negative() {
        assert!(matches!(
            rust_conversions(&context(&[(
                "a.rs",
                "fn a()->Result<(),E>{x.map_err(E::from)?; Ok(())}"
            )]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            rust_conversions(&context(&[("a.rs", "fn a(){}")])).state,
            CellState::Clean
        ));
        assert!(matches!(
            typescript_conversions(&context(&[(
                "a.ts",
                "function a(){try { x as string; } catch (error) { throw error; }}"
            )]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            typescript_conversions(&context(&[("a.ts", "function a(){}")])).state,
            CellState::Clean
        ));
    }
    #[test]
    fn typescript_and_elisp_clone_positive_and_negative() {
        assert!(matches!(
            typescript_clones(&context(&[
                ("a.ts", "function a(){const x=1; return x+1;}"),
                ("b.ts", "function b(){const y=2; return y+2;}")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            typescript_clones(&context(&[("a.ts", "function a(){}")])).state,
            CellState::Clean
        ));
        assert!(matches!(
            elisp_clones(&context(&[
                ("a.el", "(defun a () (let ((x 1)) (+ x 1)))"),
                ("b.el", "(defun b () (let ((y 2)) (+ y 2)))")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            elisp_clones(&context(&[("a.el", "(defun a () nil)")])).state,
            CellState::Clean
        ));
    }
}

//! Source and analyzer-backed census collectors.
//!
//! Collectors convert parsed source and protocol results into isolated report
//! cells. Structural extraction never claims semantic references, and missing or
//! incomplete analyzers degrade to unavailable cells while malformed input or
//! protocol failures remain explicit failures.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use oxc_allocator::Allocator;
use oxc_ast::ast::{ArrowFunctionExpression, Function};
use oxc_ast_visit::{Visit as OxcVisit, walk as oxc_walk};
use oxc_parser::{Parser, ParserReturn};
use oxc_span::Span;
use oxc_syntax::scope::ScopeFlags;
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::Visit;

use super::model::{Candidate, CellCapability, CollectorMetadata};
use super::source::language_for_path;
use super::{
    CellReport, CellState, CollectorContext, CollectorSpec, EvidenceMethod, Language, SignalFamily,
};

const STRUCTURAL_VERSION: &str = "1";

/// Registers collectors for the parsed-source and semantic minimum matrix.
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
        )
        .with_capability(CellCapability::UnreferencedExportedSymbol),
        spec(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::TypeScript,
            typescript_unreferenced_symbols,
        )
        .with_capability(CellCapability::UnreferencedExportedSymbol),
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
        capability: CellCapability::Default,
        collect,
    }
}

fn structural(
    signal: SignalFamily,
    language: Language,
    limitation: &str,
    candidates: Vec<Candidate>,
) -> CellReport {
    CellReport::candidates(
        signal,
        language,
        CollectorMetadata {
            identity: format!("census-{}-structural", language.slug()),
            version: Some(STRUCTURAL_VERSION.into()),
            evidence_method: EvidenceMethod::Structural,
            limitation: limitation.into(),
        },
        candidates,
    )
}

fn failed(signal: SignalFamily, language: Language, identity: &str, error: String) -> CellReport {
    CellReport {
        signal,
        language,
        capability: CellCapability::Default,
        collector: CollectorMetadata {
            identity: identity.into(),
            version: Some(STRUCTURAL_VERSION.into()),
            evidence_method: EvidenceMethod::Structural,
            limitation: "source could not be parsed structurally".into(),
        },
        state: CellState::Failed { error },
    }
}

fn files(context: &CollectorContext, language: Language) -> impl Iterator<Item = (&str, &str)> {
    context.snapshot.files.iter().filter_map(move |file| {
        (language_for_path(&file.path) == Some(language))
            .then_some((file.path.as_str(), file.content.as_str()))
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
            total_paths: paths.len(),
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
        Language::TypeScript
            .typescript_source_type(path)
            .expect("TypeScript language has a parser mode"),
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
            total_paths: paths.len(),
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
            total_paths: paths.len(),
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
        let shapes = match language {
            Language::Rust => rust_function_shapes(source),
            Language::TypeScript => typescript_function_shapes(path, source),
            Language::Elisp => elisp_function_shapes(source),
            Language::Repository => Ok(Vec::new()),
        };
        let shapes = match shapes {
            Ok(shapes) => shapes,
            Err(CloneShapeError::ReaderUnavailable) => {
                return CellReport::unavailable_with_collector(
                    SignalFamily::ClonesAndRepeatedTestShapes,
                    language,
                    CollectorMetadata {
                        identity: "census-elisp-structural".into(),
                        version: Some(STRUCTURAL_VERSION.into()),
                        evidence_method: EvidenceMethod::Structural,
                        limitation: "the declared Emacs reader is not available".into(),
                    },
                    "Emacs reader structural shape collector is not available",
                );
            }
            Err(CloneShapeError::Failed(error)) => {
                return failed(
                    SignalFamily::ClonesAndRepeatedTestShapes,
                    language,
                    &format!("census-{}-structural", language.slug()),
                    format!("{path}: {error}"),
                );
            }
        };
        for shape in shapes {
            groups.entry(shape).or_default().push(path.into());
        }
    }
    let candidates = groups
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(shape, paths)| Candidate {
            identity: format!("{}-shape:{:x}", language.slug(), stable_hash(&shape)),
            summary: format!(
                "repeated parsed {} function or test shape across {} files",
                language.slug(),
                paths.len()
            ),
            total_paths: paths.len(),
            paths,
        })
        .collect();
    structural(
        SignalFamily::ClonesAndRepeatedTestShapes,
        language,
        "normalized parsed function and test shapes are structural review candidates",
        candidates,
    )
}

enum CloneShapeError {
    ReaderUnavailable,
    Failed(String),
}

impl From<String> for CloneShapeError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

fn rust_function_shapes(source: &str) -> Result<Vec<String>, CloneShapeError> {
    struct Functions {
        shapes: Vec<String>,
    }
    impl<'ast> Visit<'ast> for Functions {
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            self.shapes.push(normalize_rust_shape(quote::quote!(#item)));
            syn::visit::visit_item_fn(self, item);
        }
    }
    let file =
        syn::parse_file(source).map_err(|error| CloneShapeError::Failed(error.to_string()))?;
    let mut functions = Functions { shapes: Vec::new() };
    functions.visit_file(&file);
    Ok(functions.shapes)
}

fn typescript_function_shapes(path: &str, source: &str) -> Result<Vec<String>, CloneShapeError> {
    struct Functions<'s> {
        source: &'s str,
        shapes: Vec<String>,
    }
    impl<'a> OxcVisit<'a> for Functions<'_> {
        fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
            self.shapes
                .push(normalize_typescript_span(self.source, function.span));
            oxc_walk::walk_function(self, function, flags);
        }

        fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
            self.shapes
                .push(normalize_typescript_span(self.source, function.span));
            oxc_walk::walk_arrow_function_expression(self, function);
        }
    }

    let allocator = Allocator::default();
    let ParserReturn {
        program,
        diagnostics,
        panicked,
        ..
    } = Parser::new(
        &allocator,
        source,
        Language::TypeScript
            .typescript_source_type(path)
            .expect("TypeScript language has a parser mode"),
    )
    .parse();
    if panicked || !diagnostics.is_empty() {
        return Err(diagnostics
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("; ")
            .into());
    }
    let mut functions = Functions {
        source,
        shapes: Vec::new(),
    };
    functions.visit_program(&program);
    Ok(functions.shapes)
}

fn normalize_typescript_span(source: &str, span: Span) -> String {
    normalize_shape(&source[span.start as usize..span.end as usize])
}

fn elisp_function_shapes(source: &str) -> Result<Vec<String>, CloneShapeError> {
    const READER: &str = r#"(progn
(defun census-normalize (value)
  (cond ((consp value) (cons (census-normalize (car value))
                              (census-normalize (cdr value))))
        ((symbolp value) 'id)
        ((numberp value) 'number)
        ((stringp value) 'string)
        (t 'literal)))
(with-temp-buffer
  (insert-file-contents "/dev/stdin")
  (emacs-lisp-mode)
  (check-parens)
  (goto-char (point-min))
  (condition-case nil
      (while t
        (let ((form (read (current-buffer))))
          (when (memq (car-safe form) '(defun ert-deftest))
            (princ (prin1-to-string (census-normalize form)))
            (princ "\n"))))
    (end-of-file nil))))"#;
    let mut reader = Command::new("emacs")
        .args(["--batch", "--quick", "--eval", READER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CloneShapeError::ReaderUnavailable
            } else {
                CloneShapeError::Failed(error.to_string())
            }
        })?;
    reader
        .stdin
        .take()
        .expect("configured piped stdin")
        .write_all(source.as_bytes())
        .map_err(|error| CloneShapeError::Failed(error.to_string()))?;
    let output = reader
        .wait_with_output()
        .map_err(|error| CloneShapeError::Failed(error.to_string()))?;
    if !output.status.success() {
        return Err(CloneShapeError::Failed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn normalize_rust_shape(tokens: TokenStream) -> String {
    fn tokens_into(out: &mut String, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ('(', ')'),
                        Delimiter::Brace => ('{', '}'),
                        Delimiter::Bracket => ('[', ']'),
                        Delimiter::None => ('<', '>'),
                    };
                    out.push(open);
                    tokens_into(out, group.stream());
                    out.push(close);
                }
                TokenTree::Ident(_) => out.push_str("id"),
                TokenTree::Punct(punctuation) => out.push(punctuation.as_char()),
                TokenTree::Literal(_) => out.push('#'),
            }
        }
    }

    let mut out = String::new();
    tokens_into(&mut out, tokens);
    out
}

fn normalize_shape(source: &str) -> String {
    enum State {
        Code,
        LineComment,
        BlockComment(usize),
        String(char),
    }
    let mut state = State::Code;
    let mut out = String::new();
    let mut token = String::new();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        match &mut state {
            State::Code if character == '/' && characters.peek() == Some(&'/') => {
                characters.next();
                state = State::LineComment;
            }
            State::Code if character == '/' && characters.peek() == Some(&'*') => {
                characters.next();
                state = State::BlockComment(1);
            }
            State::Code if matches!(character, '"' | '\'' | '`') => {
                normalize_token(&mut out, &mut token);
                out.push_str("str");
                state = State::String(character);
            }
            State::Code if character.is_ascii_alphanumeric() || character == '_' => {
                token.push(character)
            }
            State::Code => {
                normalize_token(&mut out, &mut token);
                if !character.is_whitespace() {
                    out.push(character);
                }
            }
            State::LineComment if character == '\n' => state = State::Code,
            State::BlockComment(depth) if character == '*' && characters.peek() == Some(&'/') => {
                characters.next();
                *depth -= 1;
                if *depth == 0 {
                    state = State::Code;
                }
            }
            State::String(_) if character == '\\' => {
                characters.next();
            }
            State::String(delimiter) if character == *delimiter => state = State::Code,
            _ => {}
        }
    }
    normalize_token(&mut out, &mut token);
    out
}

fn normalize_token(out: &mut String, token: &mut String) {
    if token.is_empty() {
        return;
    }
    out.push_str(
        if token
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            "#"
        } else {
            "id"
        },
    );
    token.clear();
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
                &format!("census-{}-structural", language.slug()),
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
                identity: format!("{}-conversion-error:{}", language.slug(), path),
                summary: format!(
                    "parsed {} conversion and error-mapping sequence",
                    language.slug()
                ),
                paths: vec![path.into()],
                total_paths: 1,
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
    .with_capability(CellCapability::UnreferencedExportedSymbol)
}
fn typescript_unreferenced_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::UnusedDependenciesAndSymbols,
        Language::TypeScript,
        "typescript-language-server",
    )
    .with_capability(CellCapability::UnreferencedExportedSymbol)
}

fn semantic_symbols(
    context: &CollectorContext,
    signal: SignalFamily,
    language: Language,
    analyzer: &str,
) -> CellReport {
    let version = match Command::new(analyzer).arg("--version").output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CellReport::unavailable_with_collector(
                signal,
                language,
                semantic_metadata(language, analyzer, None),
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
    if let Err(error) = client.open_documents(context, language) {
        return semantic_failed(signal, language, analyzer, Some(version), error);
    }
    semantic_symbols_with_session(
        context,
        signal,
        language,
        analyzer,
        Some(version),
        &mut client,
    )
}

trait SemanticSession {
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

fn semantic_symbols_with_session(
    context: &CollectorContext,
    signal: SignalFamily,
    language: Language,
    analyzer: &str,
    version: Option<String>,
    session: &mut impl SemanticSession,
) -> CellReport {
    let symbols = match session.request("workspace/symbol", serde_json::json!({ "query": "" })) {
        Ok(response) => match symbols_from_response(context, response) {
            Ok(Some(symbols)) => symbols,
            Ok(None) => {
                return semantic_unavailable(
                    signal,
                    language,
                    analyzer,
                    version.clone(),
                    "did not report export visibility",
                );
            }
            Err(error) => {
                return semantic_failed(signal, language, analyzer, version.clone(), error);
            }
        },
        Err(error) => return semantic_failed(signal, language, analyzer, version.clone(), error),
    };
    let mut candidates = Vec::new();
    let mut exported_symbols = 0;
    for symbol in symbols {
        let exported = match session.request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": symbol.uri },
                "position": { "line": symbol.line, "character": symbol.character }
            }),
        ) {
            Ok(response) => hover_reports_exported(language, response),
            Err(error) => {
                return semantic_failed(signal, language, analyzer, version.clone(), error);
            }
        };
        if !exported {
            continue;
        }
        exported_symbols += 1;
        let references = match session.request(
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
                    return semantic_failed(signal, language, analyzer, version.clone(), error);
                }
            },
            Err(error) => {
                return semantic_failed(signal, language, analyzer, version.clone(), error);
            }
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
                total_paths: paths.len(),
                paths,
            });
        }
    }
    if exported_symbols == 0 {
        return semantic_unavailable(
            signal,
            language,
            analyzer,
            version,
            "did not report export visibility",
        );
    }
    CellReport::candidates(
        signal,
        language,
        semantic_metadata(language, analyzer, version),
        candidates,
    )
}

fn semantic_metadata(
    language: Language,
    analyzer: &str,
    version: Option<String>,
) -> CollectorMetadata {
    CollectorMetadata {
        identity: format!("census-{}-semantic-lsp", language.slug()),
        version,
        evidence_method: EvidenceMethod::Semantic,
        limitation: format!(
            "requires `{analyzer}` workspace symbols, export visibility, and references from its declared project"
        ),
    }
}

fn semantic_unavailable(
    signal: SignalFamily,
    language: Language,
    analyzer: &str,
    version: Option<String>,
    reason: &str,
) -> CellReport {
    CellReport::unavailable_with_collector(
        signal,
        language,
        semantic_metadata(language, analyzer, version),
        format!("semantic analyzer `{analyzer}` {reason}"),
    )
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
    stderr: std::process::ChildStderr,
    responses: Receiver<Result<serde_json::Value, String>>,
    next_id: u64,
}

impl Drop for LspClient {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) = self.child.kill() {
                    eprintln!("warning: stopping census LSP server failed: {error}");
                }
                if let Err(error) = self.child.wait() {
                    eprintln!("warning: reaping census LSP server failed: {error}");
                }
            }
            Err(error) => {
                eprintln!("warning: checking census LSP server status failed: {error}");
                if let Err(error) = self.child.wait() {
                    eprintln!(
                        "warning: reaping census LSP server after status failure failed: {error}"
                    );
                }
            }
        }
        let mut stderr = String::new();
        if let Err(error) = self.stderr.read_to_string(&mut stderr) {
            eprintln!("warning: reading census LSP server stderr during cleanup failed: {error}");
        } else if !stderr.trim().is_empty() {
            eprintln!(
                "warning: census LSP server stderr during cleanup: {}",
                stderr.trim()
            );
        }
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
            .stderr(Stdio::piped());
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
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| format!("`{analyzer}` did not provide LSP stderr"))?;
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
            stderr,
            responses,
            next_id: 1,
        };
        let root = absolute_repo_root(context)?;
        let root_uri = url::Url::from_directory_path(&root)
            .map_err(|_| format!("cannot form LSP root URI for {}", root.display()))?
            .to_string();
        client.request(
            "initialize",
            serde_json::json!({
                "processId": null,
                "rootUri": root_uri.clone(),
                "workspaceFolders": [{ "uri": root_uri, "name": "repository" }],
                "capabilities": {
                    "workspace": { "symbol": {}, "workspaceFolders": true },
                    "textDocument": { "references": {} }
                }
            }),
        )?;
        client.notify("initialized", serde_json::json!({}))?;
        Ok(client)
    }

    fn open_documents(
        &mut self,
        context: &CollectorContext,
        language: Language,
    ) -> Result<(), String> {
        for params in lsp_open_document_params(context, language)? {
            self.notify("textDocument/didOpen", params)?;
        }
        Ok(())
    }

    fn lsp_language_id(language: Language, path: &str) -> Option<&'static str> {
        language.lsp_language_id(path)
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

fn lsp_open_document_params(
    context: &CollectorContext,
    language: Language,
) -> Result<Vec<serde_json::Value>, String> {
    let root = absolute_repo_root(context)?;
    context
        .snapshot
        .files
        .iter()
        .filter_map(|file| {
            LspClient::lsp_language_id(language, &file.path).map(|language_id| (file, language_id))
        })
        .map(|(file, language_id)| {
            let uri = url::Url::from_file_path(root.join(&file.path))
                .map_err(|_| format!("cannot form LSP document URI for {}", file.path))?
                .to_string();
            Ok(serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": file.content,
                }
            }))
        })
        .collect()
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
impl SemanticSession for LspClient {
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        LspClient::request(self, method, params)
    }
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
) -> Result<Option<Vec<SemanticSymbol>>, String> {
    if response.is_null() {
        return Ok(None);
    }
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
    Ok(Some(symbols))
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
        capability: CellCapability::Default,
        collector: semantic_metadata(language, analyzer, version),
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
    use std::collections::VecDeque;
    struct FixtureLspSession {
        responses: VecDeque<(&'static str, serde_json::Value)>,
    }

    impl SemanticSession for FixtureLspSession {
        fn request(
            &mut self,
            method: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            let (expected_method, response) = self
                .responses
                .pop_front()
                .ok_or_else(|| format!("unexpected LSP request `{method}`"))?;
            if method != expected_method {
                return Err(format!(
                    "expected LSP request `{expected_method}`, got `{method}`"
                ));
            }
            Ok(response)
        }
    }

    fn standard_lsp_session(references: serde_json::Value) -> FixtureLspSession {
        FixtureLspSession {
            responses: VecDeque::from([
                (
                    "workspace/symbol",
                    serde_json::json!([{
                        "name": "public_api",
                        "kind": 12,
                        "location": {
                            "uri": "file:///repo/src/lib.rs",
                            "range": {
                                "start": { "line": 4, "character": 7 },
                                "end": { "line": 4, "character": 17 }
                            }
                        }
                    }]),
                ),
                (
                    "textDocument/hover",
                    serde_json::json!({
                        "contents": { "kind": "markdown", "value": "```rust\npub fn public_api()\n```" }
                    }),
                ),
                ("textDocument/references", references),
            ]),
        }
    }

    #[test]
    fn semantic_collectors_classify_standard_lsp_reference_results_through_the_session_seam() {
        let context = CollectorContext {
            repo_root: "/repo".into(),
            snapshot: SourceSnapshot::default(),
        };
        let referenced = serde_json::json!([{
            "uri": "file:///repo/src/caller.rs",
            "range": {
                "start": { "line": 8, "character": 2 },
                "end": { "line": 8, "character": 12 }
            }
        }]);
        let unreferenced = serde_json::json!([]);

        let mut exported_positive = standard_lsp_session(referenced.clone());
        let report = semantic_symbols_with_session(
            &context,
            SignalFamily::ExportedSymbolReferences,
            Language::Rust,
            "fixture-lsp",
            Some("1.0".into()),
            &mut exported_positive,
        );
        let CellState::Candidates { candidates, .. } = report.state else {
            panic!("a standard LSP reference must be an exported-reference candidate");
        };
        assert_eq!(candidates[0].paths, ["src/caller.rs", "src/lib.rs"]);
        assert_eq!(report.collector.evidence_method, EvidenceMethod::Semantic);

        let mut exported_clean = standard_lsp_session(unreferenced.clone());
        assert!(matches!(
            semantic_symbols_with_session(
                &context,
                SignalFamily::ExportedSymbolReferences,
                Language::Rust,
                "fixture-lsp",
                Some("1.0".into()),
                &mut exported_clean,
            )
            .state,
            CellState::Clean
        ));

        let mut unreferenced_positive = standard_lsp_session(unreferenced);
        assert!(matches!(
            semantic_symbols_with_session(
                &context,
                SignalFamily::UnusedDependenciesAndSymbols,
                Language::Rust,
                "fixture-lsp",
                Some("1.0".into()),
                &mut unreferenced_positive,
            )
            .state,
            CellState::Candidates { .. }
        ));

        let mut unreferenced_clean = standard_lsp_session(referenced);
        assert!(matches!(
            semantic_symbols_with_session(
                &context,
                SignalFamily::UnusedDependenciesAndSymbols,
                Language::Rust,
                "fixture-lsp",
                Some("1.0".into()),
                &mut unreferenced_clean,
            )
            .state,
            CellState::Clean
        ));
    }
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
        let symbols = symbols.expect("array result contains symbols");
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

        assert!(
            symbols_from_response(&context, serde_json::Value::Null)
                .expect("null workspace/symbol response is valid")
                .is_none()
        );
        assert!(symbols_from_response(&context, serde_json::json!({})).is_err());
        assert!(references_from_response(&context, serde_json::json!({})).is_err());
    }
    #[test]
    fn typescript_project_documents_use_standard_did_open_parameters() {
        let context = CollectorContext {
            repo_root: "/repo".into(),
            snapshot: SourceSnapshot {
                files: vec![
                    SourceFile {
                        path: "end2end/tests/public-api.ts".into(),
                        content: "export const api = 1;".into(),
                    },
                    SourceFile {
                        path: "end2end/tests/view.tsx".into(),
                        content: "export const View = () => null;".into(),
                    },
                    SourceFile {
                        path: "server/src/lib.rs".into(),
                        content: "pub fn ignored() {}".into(),
                    },
                ],
            },
        };
        let documents = lsp_open_document_params(&context, Language::TypeScript)
            .expect("forms didOpen payloads");
        assert_eq!(documents.len(), 2);
        assert_eq!(
            documents[0],
            serde_json::json!({
                "textDocument": {
                    "uri": "file:///repo/end2end/tests/public-api.ts",
                    "languageId": "typescript",
                    "version": 1,
                    "text": "export const api = 1;",
                }
            })
        );
        assert_eq!(
            documents[1]["textDocument"]["languageId"],
            serde_json::json!("typescriptreact")
        );
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
    fn missing_semantic_analyzer_is_unavailable_with_semantic_provenance() {
        let report = semantic_symbols(
            &context(&[]),
            SignalFamily::ExportedSymbolReferences,
            Language::Rust,
            "jaunder-census-definitely-missing-analyzer",
        );
        assert!(matches!(report.state, CellState::Unavailable { .. }));
        assert_eq!(report.collector.identity, "census-rust-semantic-lsp");
        assert_eq!(report.collector.evidence_method, EvidenceMethod::Semantic);
        assert_eq!(report.collector.version, None);
        assert!(report.collector.limitation.contains("declared project"));
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
    fn clone_collectors_use_parsed_function_shapes_and_ignore_markers_in_comments_and_strings() {
        let typescript = typescript_clones(&context(&[
            (
                "a.ts",
                "function alpha() { /* CLONE-A */ return \"CLONE-A\"; }",
            ),
            (
                "b.ts",
                "function beta() { /* CLONE-B */ return \"CLONE-B\"; }",
            ),
        ]));
        let CellState::Candidates { candidates, .. } = typescript.state else {
            panic!("parsed TypeScript functions with equivalent structure are candidates");
        };
        assert_eq!(candidates[0].paths, ["a.ts", "b.ts"]);
        assert_eq!(
            typescript.collector.evidence_method,
            EvidenceMethod::Structural
        );

        let clean = typescript_clones(&context(&[
            ("a.ts", "function alpha() { return 1; }"),
            ("b.ts", "function beta() { return 1; return 2; }"),
        ]));
        assert!(matches!(clean.state, CellState::Clean));

        let malformed = typescript_clones(&context(&[("a.ts", "function broken( {")]));
        assert!(matches!(malformed.state, CellState::Failed { .. }));
    }

    #[test]
    fn elisp_clone_collector_reads_top_level_functions_and_tests() {
        let report = elisp_clones(&context(&[
            (
                "a.el",
                "(defun alpha () (message \"CLONE-A\"))\n(ert-deftest alpha-test () (should t))",
            ),
            (
                "b.el",
                "(defun beta () (message \"CLONE-B\"))\n(ert-deftest beta-test () (should t))",
            ),
        ]));
        match report.state {
            CellState::Candidates { candidates, .. } => {
                assert_eq!(candidates.len(), 2);
                assert!(
                    candidates
                        .iter()
                        .all(|candidate| candidate.paths == ["a.el", "b.el"])
                );
            }
            CellState::Unavailable { .. } => {
                assert!(report.collector.limitation.contains("Emacs reader"));
            }
            state => {
                panic!("top-level Elisp forms must be reader-derived candidates, got {state:?}")
            }
        }
        assert_eq!(report.collector.evidence_method, EvidenceMethod::Structural);
    }
}

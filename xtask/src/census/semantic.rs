//! Semantic collection policy over language-server sessions.

use std::process::Command;

use super::lsp::{
    LspClient, hover_reports_exported, references_from_response, symbols_from_response,
};
use super::model::{Candidate, CellCapability, CollectorMetadata};
use super::{CellReport, CellState, CollectorContext, EvidenceMethod, Language, SignalFamily};

pub(crate) fn rust_semantic_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::ExportedSymbolReferences,
        Language::Rust,
        "rust-analyzer",
    )
}
pub(crate) fn typescript_semantic_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::ExportedSymbolReferences,
        Language::TypeScript,
        "typescript-language-server",
    )
}
pub(crate) fn rust_unreferenced_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::UnusedDependenciesAndSymbols,
        Language::Rust,
        "rust-analyzer",
    )
    .with_capability(CellCapability::UnreferencedExportedSymbol)
}
pub(crate) fn typescript_unreferenced_symbols(context: &CollectorContext) -> CellReport {
    semantic_symbols(
        context,
        SignalFamily::UnusedDependenciesAndSymbols,
        Language::TypeScript,
        "typescript-language-server",
    )
    .with_capability(CellCapability::UnreferencedExportedSymbol)
}

pub(crate) fn semantic_symbols(
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

pub(crate) trait SemanticSession {
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

pub(crate) fn semantic_symbols_with_session(
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::super::SourceSnapshot;
    use super::*;

    struct FixtureSession {
        responses: VecDeque<(&'static str, serde_json::Value)>,
    }

    impl SemanticSession for FixtureSession {
        fn request(
            &mut self,
            method: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            let (expected, response) = self
                .responses
                .pop_front()
                .ok_or_else(|| format!("unexpected LSP request `{method}`"))?;
            if method != expected {
                return Err(format!("expected LSP request `{expected}`, got `{method}`"));
            }
            Ok(response)
        }
    }

    fn session(references: serde_json::Value) -> FixtureSession {
        FixtureSession {
            responses: VecDeque::from([
                (
                    "workspace/symbol",
                    serde_json::json!([{
                        "name": "public_api",
                        "location": {
                            "uri": "file:///repo/src/lib.rs",
                            "range": { "start": { "line": 4, "character": 7 } }
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

    fn context() -> CollectorContext {
        CollectorContext {
            repo_root: "/repo".into(),
            snapshot: SourceSnapshot::default(),
        }
    }

    #[test]
    fn null_lsp_references_are_an_empty_reference_set_for_both_semantic_signals() {
        let mut exported = session(serde_json::Value::Null);
        assert!(matches!(
            semantic_symbols_with_session(
                &context(),
                SignalFamily::ExportedSymbolReferences,
                Language::Rust,
                "fixture-lsp",
                Some("1".into()),
                &mut exported,
            )
            .state,
            CellState::Clean
        ));

        let mut unreferenced = session(serde_json::Value::Null);
        assert!(matches!(
            semantic_symbols_with_session(
                &context(),
                SignalFamily::UnusedDependenciesAndSymbols,
                Language::Rust,
                "fixture-lsp",
                Some("1".into()),
                &mut unreferenced,
            )
            .state,
            CellState::Candidates { .. }
        ));
    }

    #[test]
    fn malformed_non_null_lsp_references_fail_the_cell() {
        let mut malformed = session(serde_json::json!({}));
        assert!(matches!(
            semantic_symbols_with_session(
                &context(),
                SignalFamily::ExportedSymbolReferences,
                Language::Rust,
                "fixture-lsp",
                Some("1".into()),
                &mut malformed,
            )
            .state,
            CellState::Failed { .. }
        ));
    }
}

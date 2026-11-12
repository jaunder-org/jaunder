//! Semantic collection policy over one language-server scan per command language.
//!
//! This module projects analyzer facts into the independently keyed exported-reference
//! and unreferenced-export cells. A command-scoped context memoizes that paired
//! projection, so each language opens one analyzer session and asks for workspace
//! symbols, export visibility, and references once. LSP evidence remains bounded
//! by the analyzer's declared project and its visibility semantics; unavailable
//! analyzers, incomplete export data, and protocol failures are reported rather
//! than inferred as clean. Session lifecycle and stderr draining belong to `lsp`.

use std::process::Command;

use super::lsp::{
    LspClient, hover_reports_exported, references_from_response, symbols_from_response,
};
use super::model::{Candidate, CellCapability, CollectorMetadata};
use super::{CellReport, CellState, CollectorContext, EvidenceMethod, Language, SignalFamily};

pub(crate) fn rust_semantic_symbols(context: &CollectorContext) -> CellReport {
    semantic_report(context, Language::Rust, "rust-analyzer", false)
}

pub(crate) fn typescript_semantic_symbols(context: &CollectorContext) -> CellReport {
    semantic_report(
        context,
        Language::TypeScript,
        "typescript-language-server",
        false,
    )
}

pub(crate) fn rust_unreferenced_symbols(context: &CollectorContext) -> CellReport {
    semantic_report(context, Language::Rust, "rust-analyzer", true)
}

pub(crate) fn typescript_unreferenced_symbols(context: &CollectorContext) -> CellReport {
    semantic_report(
        context,
        Language::TypeScript,
        "typescript-language-server",
        true,
    )
}

fn semantic_report(
    context: &CollectorContext,
    language: Language,
    analyzer: &str,
    unreferenced: bool,
) -> CellReport {
    let reports = {
        let cache = context
            .semantic_reports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.get(&language).cloned()
    };
    let reports = reports.unwrap_or_else(|| {
        let reports = scan_language(context, language, analyzer);
        let mut cache = context
            .semantic_reports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache
            .entry(language)
            .or_insert_with(|| reports.clone())
            .clone()
    });
    if unreferenced { reports.1 } else { reports.0 }
}

fn scan_language(
    context: &CollectorContext,
    language: Language,
    analyzer: &str,
) -> (CellReport, CellReport) {
    let version = match Command::new(analyzer).arg("--version").output() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return unavailable_pair(
                language,
                analyzer,
                None,
                format!("semantic analyzer `{analyzer}` is not installed"),
            );
        }
        Err(error) => {
            return failed_pair(
                language,
                analyzer,
                None,
                format!("cannot execute semantic analyzer: {error}"),
            );
        }
        Ok(output) if !output.status.success() => {
            return failed_pair(
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
        Err(error) => return failed_pair(language, analyzer, Some(version), error),
    };
    if let Err(error) = client.open_documents(context, language) {
        return failed_pair(language, analyzer, Some(version), error);
    }
    semantic_reports_with_session(context, language, analyzer, Some(version), &mut client)
}

pub(crate) trait SemanticSession {
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

fn semantic_reports_with_session(
    context: &CollectorContext,
    language: Language,
    analyzer: &str,
    version: Option<String>,
    session: &mut impl SemanticSession,
) -> (CellReport, CellReport) {
    let symbols = match scan_symbols(context, language, session) {
        Ok(symbols) => symbols,
        Err(ScanFailure::Unavailable(reason)) => {
            return unavailable_pair(language, analyzer, version, reason);
        }
        Err(ScanFailure::Failed(error)) => return failed_pair(language, analyzer, version, error),
    };
    let mut referenced = Vec::new();
    let mut unreferenced = Vec::new();
    for symbol in symbols {
        let candidate = Candidate {
            identity: format!("semantic-symbol:{}", symbol.name),
            summary: format!(
                "LSP reference analysis for exported symbol `{}`",
                symbol.name
            ),
            total_paths: symbol.references.len() + 1,
            paths: {
                let mut paths = symbol.references;
                paths.push(symbol.path);
                paths
            },
        };
        if candidate.total_paths == 1 {
            unreferenced.push(candidate);
        } else {
            referenced.push(candidate);
        }
    }
    (
        CellReport::candidates(
            SignalFamily::ExportedSymbolReferences,
            language,
            semantic_metadata(language, analyzer, version.clone()),
            referenced,
        ),
        CellReport::candidates(
            SignalFamily::UnusedDependenciesAndSymbols,
            language,
            semantic_metadata(language, analyzer, version),
            unreferenced,
        )
        .with_capability(CellCapability::UnreferencedExportedSymbol),
    )
}

struct SemanticFact {
    name: String,
    path: String,
    references: Vec<String>,
}

enum ScanFailure {
    Unavailable(String),
    Failed(String),
}

fn scan_symbols(
    context: &CollectorContext,
    language: Language,
    session: &mut impl SemanticSession,
) -> Result<Vec<SemanticFact>, ScanFailure> {
    let symbols = match session.request("workspace/symbol", serde_json::json!({ "query": "" })) {
        Ok(response) => match symbols_from_response(context, response) {
            Ok(Some(symbols)) => symbols,
            Ok(None) => {
                return Err(ScanFailure::Unavailable(
                    "did not report export visibility".into(),
                ));
            }
            Err(error) => return Err(ScanFailure::Failed(error)),
        },
        Err(error) => return Err(ScanFailure::Failed(error)),
    };
    let mut facts = Vec::new();
    for symbol in symbols {
        let exported = session
            .request(
                "textDocument/hover",
                serde_json::json!({
                    "textDocument": { "uri": symbol.uri },
                    "position": { "line": symbol.line, "character": symbol.character }
                }),
            )
            .map_err(ScanFailure::Failed)
            .map(|response| hover_reports_exported(language, response))?;
        if !exported {
            continue;
        }
        let references = session
            .request(
                "textDocument/references",
                serde_json::json!({
                    "textDocument": { "uri": symbol.uri },
                    "position": { "line": symbol.line, "character": symbol.character },
                    "context": { "includeDeclaration": false }
                }),
            )
            .map_err(ScanFailure::Failed)
            .and_then(|response| {
                references_from_response(context, response).map_err(ScanFailure::Failed)
            })?;
        facts.push(SemanticFact {
            name: symbol.name,
            path: symbol.path,
            references,
        });
    }
    if facts.is_empty() {
        return Err(ScanFailure::Unavailable(
            "did not report export visibility".into(),
        ));
    }
    Ok(facts)
}

#[cfg(test)]
fn semantic_symbols_with_session(
    context: &CollectorContext,
    signal: SignalFamily,
    language: Language,
    analyzer: &str,
    version: Option<String>,
    session: &mut impl SemanticSession,
) -> CellReport {
    let reports = semantic_reports_with_session(context, language, analyzer, version, session);
    match signal {
        SignalFamily::ExportedSymbolReferences => reports.0,
        SignalFamily::UnusedDependenciesAndSymbols => reports.1,
        _ => failed_pair(language, analyzer, None, "invalid semantic signal".into()).0,
    }
}

fn unavailable_pair(
    language: Language,
    analyzer: &str,
    version: Option<String>,
    reason: String,
) -> (CellReport, CellReport) {
    (
        semantic_unavailable(
            SignalFamily::ExportedSymbolReferences,
            language,
            analyzer,
            version.clone(),
            &reason,
        ),
        semantic_unavailable(
            SignalFamily::UnusedDependenciesAndSymbols,
            language,
            analyzer,
            version,
            &reason,
        )
        .with_capability(CellCapability::UnreferencedExportedSymbol),
    )
}

fn failed_pair(
    language: Language,
    analyzer: &str,
    version: Option<String>,
    error: String,
) -> (CellReport, CellReport) {
    (
        semantic_failed(
            SignalFamily::ExportedSymbolReferences,
            language,
            analyzer,
            version.clone(),
            error.clone(),
        ),
        semantic_failed(
            SignalFamily::UnusedDependenciesAndSymbols,
            language,
            analyzer,
            version,
            error,
        )
        .with_capability(CellCapability::UnreferencedExportedSymbol),
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

    fn session(language: Language, references: serde_json::Value) -> FixtureSession {
        let (path, hover) = match language {
            Language::Rust => (
                "src/lib.rs",
                serde_json::json!({
                    "contents": { "kind": "markdown", "value": "```rust\npub fn public_api()\n```" }
                }),
            ),
            Language::TypeScript => (
                "end2end/public-api.ts",
                serde_json::json!({
                    "contents": { "kind": "markdown", "value": "```typescript\nexport declare function public_api(): void\n```" }
                }),
            ),
            Language::Elisp | Language::Repository => unreachable!("semantic fixture language"),
        };
        FixtureSession {
            responses: VecDeque::from([
                (
                    "workspace/symbol",
                    serde_json::json!([{
                        "name": "public_api",
                        "location": {
                            "uri": format!("file:///repo/{path}"),
                            "range": { "start": { "line": 4, "character": 7 } }
                        }
                    }]),
                ),
                ("textDocument/hover", hover),
                ("textDocument/references", references),
            ]),
        }
    }

    fn context() -> CollectorContext {
        CollectorContext {
            repo_root: "/repo".into(),
            snapshot: SourceSnapshot::default(),
            semantic_reports: Default::default(),
        }
    }

    #[test]
    fn one_session_scan_projects_both_semantic_cells() {
        let mut fixture = session(
            Language::Rust,
            serde_json::json!([{
                "uri": "file:///repo/src/consumer.rs",
                "range": { "start": { "line": 1, "character": 0 } }
            }]),
        );
        let (exported, unused) = semantic_reports_with_session(
            &context(),
            Language::Rust,
            "fixture-lsp",
            Some("1".into()),
            &mut fixture,
        );
        assert!(matches!(exported.state, CellState::Candidates { .. }));
        assert!(matches!(unused.state, CellState::Clean));
        assert!(fixture.responses.is_empty());
    }
    #[test]
    fn semantic_reference_fixtures_cover_positive_and_clean_cells_for_both_languages() {
        for language in [Language::Rust, Language::TypeScript] {
            let referenced_path = match language {
                Language::Rust => "file:///repo/src/consumer.rs",
                Language::TypeScript => "file:///repo/end2end/consumer.ts",
                Language::Elisp | Language::Repository => unreachable!("semantic fixture language"),
            };
            let references = serde_json::json!([{
                "uri": referenced_path,
                "range": { "start": { "line": 1, "character": 0 } }
            }]);
            let mut exported = session(language, references.clone());
            assert!(matches!(
                semantic_symbols_with_session(
                    &context(),
                    SignalFamily::ExportedSymbolReferences,
                    language,
                    "fixture-lsp",
                    Some("1".into()),
                    &mut exported,
                )
                .state,
                CellState::Candidates { .. }
            ));

            let mut unreferenced = session(language, references);
            assert!(matches!(
                semantic_symbols_with_session(
                    &context(),
                    SignalFamily::UnusedDependenciesAndSymbols,
                    language,
                    "fixture-lsp",
                    Some("1".into()),
                    &mut unreferenced,
                )
                .state,
                CellState::Clean
            ));
        }
    }

    #[test]
    fn null_and_empty_references_are_clean_exported_references_and_unreferenced_candidates() {
        for language in [Language::Rust, Language::TypeScript] {
            for references in [serde_json::Value::Null, serde_json::json!([])] {
                let mut exported = session(language, references.clone());
                assert!(matches!(
                    semantic_symbols_with_session(
                        &context(),
                        SignalFamily::ExportedSymbolReferences,
                        language,
                        "fixture-lsp",
                        Some("1".into()),
                        &mut exported,
                    )
                    .state,
                    CellState::Clean
                ));

                let mut unreferenced = session(language, references);
                assert!(matches!(
                    semantic_symbols_with_session(
                        &context(),
                        SignalFamily::UnusedDependenciesAndSymbols,
                        language,
                        "fixture-lsp",
                        Some("1".into()),
                        &mut unreferenced,
                    )
                    .state,
                    CellState::Candidates { .. }
                ));
            }
        }
    }

    #[test]
    fn malformed_non_null_lsp_references_fail_the_cell() {
        let mut malformed = session(Language::Rust, serde_json::json!({}));
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

//! Dependency-structure collectors over tracked source syntax.
//!
//! This module groups parsed Rust `use`, TypeScript import/export, and Elisp
//! require/provide forms into structural dependency candidates. It neither
//! resolves package graphs nor proves runtime dependency edges, so its evidence
//! is deliberately limited to source syntax. Parse failures are failed cells and
//! malformed Elisp remains explicit rather than clean; orchestration owns report
//! ordering and command lifecycle.

use std::collections::{BTreeMap, BTreeSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_parser::{Parser, ParserReturn};
use syn::visit::Visit;

use super::common::{STRUCTURAL_VERSION, failed, files, structural};
use super::elisp::{self, ReaderError};
use super::model::{Candidate, CollectorMetadata};
use super::{CellReport, CollectorContext, EvidenceMethod, Language, SignalFamily};
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

pub(crate) fn rust_dependencies(context: &CollectorContext) -> CellReport {
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

pub(crate) fn typescript_dependencies(context: &CollectorContext) -> CellReport {
    let mut imports = BTreeMap::<String, Vec<String>>::new();
    for (path, source) in files(context, Language::TypeScript) {
        let modules = match typescript_modules(path, source) {
            Ok(modules) => modules,
            Err(error) => {
                return failed(
                    SignalFamily::DependencyStructure,
                    Language::TypeScript,
                    "census-typescript-structural",
                    format!("{path}: {error}"),
                );
            }
        };
        for module in modules {
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
        "imports and re-exports are extracted from Oxc declarations; aliases and runtime resolution are not modeled",
        candidates,
    )
}

fn typescript_modules(path: &str, source: &str) -> Result<Vec<String>, String> {
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
            .join("; "));
    }
    Ok(program
        .body
        .iter()
        .filter_map(|statement| match statement {
            Statement::ImportDeclaration(declaration) => Some(declaration.source.value.as_str()),
            Statement::ExportFromDeclaration(declaration) => {
                Some(declaration.source.value.as_str())
            }
            Statement::ExportAllDeclaration(declaration) => Some(declaration.source.value.as_str()),
            _ => None,
        })
        .map(str::to_owned)
        .collect())
}

trait ElispReader {
    fn top_level_dependencies(&mut self, source: &str) -> Result<Vec<String>, ReaderError>;
}

struct EmacsElispReader;

impl ElispReader for EmacsElispReader {
    fn top_level_dependencies(&mut self, source: &str) -> Result<Vec<String>, ReaderError> {
        elisp::top_level_dependencies(source)
    }
}

pub(crate) fn elisp_dependencies(context: &CollectorContext) -> CellReport {
    elisp_dependencies_with_reader(context, &mut EmacsElispReader)
}

fn elisp_dependencies_with_reader(
    context: &CollectorContext,
    reader: &mut impl ElispReader,
) -> CellReport {
    let mut dependencies = BTreeMap::<String, Vec<String>>::new();
    for (path, source) in files(context, Language::Elisp) {
        let source_dependencies = match reader.top_level_dependencies(source) {
            Ok(dependencies) => dependencies,
            Err(ReaderError::Unavailable) => {
                return CellReport::unavailable_with_collector(
                    SignalFamily::DependencyStructure,
                    Language::Elisp,
                    CollectorMetadata {
                        identity: "census-elisp-structural".into(),
                        version: Some(STRUCTURAL_VERSION.into()),
                        evidence_method: EvidenceMethod::Structural,
                        limitation: "the declared Emacs reader is not available".into(),
                    },
                    "Emacs reader structural dependency collector is not available",
                );
            }
            Err(ReaderError::Failed(error)) => {
                return failed(
                    SignalFamily::DependencyStructure,
                    Language::Elisp,
                    "census-elisp-structural",
                    format!("{path}: {error}"),
                );
            }
        };
        for dependency in source_dependencies {
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
#[cfg(test)]
mod tests {
    use super::super::CellState;
    use super::super::common::context;
    use super::*;
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
    fn typescript_dependency_multiline_imports_and_reexports_are_structural() {
        assert!(matches!(
            typescript_dependencies(&context(&[
                ("a.ts", "import {\n  alpha,\n} from 'pkg';"),
                ("b.ts", "export {\n  beta,\n} from 'pkg';")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            typescript_dependencies(&context(&[
                ("a.ts", "import {\n  alpha,\n} from 'a';"),
                ("b.ts", "export *\n  from 'b';")
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
    fn elisp_dependency_reader_ignores_parentheses_in_strings_and_comments() {
        // Reader-valid lexical parentheses must not hide a real top-level dependency.
        let report = elisp_dependencies(&context(&[
            ("a.el", "\"((\"\n; ((\n(require\n 'dash)\n"),
            ("b.el", "(require 'dash)"),
        ]));

        let CellState::Candidates { candidates, .. } = report.state else {
            panic!("expected a dependency candidate");
        };
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].identity, "elisp-dependency:dash");
        assert_eq!(candidates[0].paths, ["a.el", "b.el"]);
    }

    #[test]
    fn elisp_dependency_reader_ignores_non_code_and_nested_requires() {
        // Dependency evidence is limited to literal quoted require forms at top level.
        let report = elisp_dependencies(&context(&[(
            "a.el",
            r#"; (require 'dash)
"(require 'dash)"
(when t (require 'dash))
(require feature)
(require (intern "dash"))
"#,
        )]));

        assert_eq!(report.state, CellState::Clean);
    }

    #[test]
    fn malformed_elisp_dependency_source_fails_with_its_path() {
        // A reader failure must remain visible and attributable to the source file.
        let report = elisp_dependencies(&context(&[("broken.el", "(require 'dash")]));

        let CellState::Failed { error } = report.state else {
            panic!("expected a failed dependency cell");
        };
        assert!(error.starts_with("broken.el: "));
    }

    #[test]
    fn unavailable_elisp_reader_produces_unavailable_cell() {
        struct UnavailableReader;

        impl ElispReader for UnavailableReader {
            fn top_level_dependencies(&mut self, _: &str) -> Result<Vec<String>, ReaderError> {
                Err(ReaderError::Unavailable)
            }
        }

        // Missing declared tooling is unavailable rather than inferred clean or failed.
        let report = elisp_dependencies_with_reader(
            &context(&[("a.el", "(require 'dash)")]),
            &mut UnavailableReader,
        );

        assert!(matches!(report.state, CellState::Unavailable { .. }));
        assert_eq!(report.collector.identity, "census-elisp-structural");
    }
}

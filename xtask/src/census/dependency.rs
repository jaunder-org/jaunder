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

use super::common::{balanced_elisp, failed, files, structural};
use super::model::Candidate;
use super::{CellReport, CollectorContext, Language, SignalFamily};
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

pub(crate) fn elisp_dependencies(context: &CollectorContext) -> CellReport {
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
}

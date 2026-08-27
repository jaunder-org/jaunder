//! Central census collector registry.

use super::clone::{elisp_clones, rust_clones, typescript_clones};
use super::conversion::{rust_conversions, typescript_conversions};
use super::dependency::{elisp_dependencies, rust_dependencies, typescript_dependencies};
use super::semantic::{
    rust_semantic_symbols, rust_unreferenced_symbols, typescript_semantic_symbols,
    typescript_unreferenced_symbols,
};
use super::{CellCapability, CellReport, CollectorContext, CollectorSpec, Language, SignalFamily};

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

pub(crate) fn specs() -> Vec<CollectorSpec> {
    let mut specs = vec![
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
    ];
    specs.extend(super::history::specs());
    specs.extend(super::adapters::specs());
    specs
}

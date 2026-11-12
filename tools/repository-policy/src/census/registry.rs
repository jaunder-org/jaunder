//! Central census cell catalog.
//!
//! Each entry names a required report cell, its optional collector, and the
//! reason reported when that collector is unavailable. Consumers derive both
//! initial report cells and collector execution from this catalog.

use super::clone::{elisp_clones, rust_clones, typescript_clones};
use super::conversion::{rust_conversions, typescript_conversions};
use super::dependency::{elisp_dependencies, rust_dependencies, typescript_dependencies};
use super::semantic::{
    rust_semantic_symbols, rust_unreferenced_symbols, typescript_semantic_symbols,
    typescript_unreferenced_symbols,
};
use super::{CellCapability, CellSpec, Language, SignalFamily};

const fn cell(
    signal: SignalFamily,
    language: Language,
    capability: CellCapability,
    collect: Option<fn(&super::CollectorContext) -> super::CellReport>,
    unavailable_reason: &'static str,
) -> CellSpec {
    CellSpec {
        signal,
        language,
        capability,
        collect,
        unavailable_reason,
    }
}

/// The complete, deterministic census surface.
pub(crate) fn catalog() -> &'static [CellSpec] {
    const DEPENDENCY_UNAVAILABLE: &str = "dependency collector is not available";
    const SEMANTIC_UNAVAILABLE: &str = "semantic analyzer is not available";
    const CLONE_UNAVAILABLE: &str = "structural clone collector is not available";
    const CONVERSION_UNAVAILABLE: &str = "structural conversion collector is not available";
    const UNUSED_DEPENDENCY_UNAVAILABLE: &str =
        "sound unused-dependency collector is not available";
    const HISTORY_UNAVAILABLE: &str = "history collector is not available";
    const ADAPTER_UNAVAILABLE: &str = "SQLite/PostgreSQL adapter-path collector is not available";

    static CATALOG: [CellSpec; 17] = [
        cell(
            SignalFamily::DependencyStructure,
            Language::Rust,
            CellCapability::Default,
            Some(rust_dependencies),
            DEPENDENCY_UNAVAILABLE,
        ),
        cell(
            SignalFamily::DependencyStructure,
            Language::TypeScript,
            CellCapability::Default,
            Some(typescript_dependencies),
            DEPENDENCY_UNAVAILABLE,
        ),
        cell(
            SignalFamily::DependencyStructure,
            Language::Elisp,
            CellCapability::Default,
            Some(elisp_dependencies),
            DEPENDENCY_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ExportedSymbolReferences,
            Language::Rust,
            CellCapability::Default,
            Some(rust_semantic_symbols),
            SEMANTIC_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ExportedSymbolReferences,
            Language::TypeScript,
            CellCapability::Default,
            Some(typescript_semantic_symbols),
            SEMANTIC_UNAVAILABLE,
        ),
        cell(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::Rust,
            CellCapability::UnusedDependency,
            None,
            UNUSED_DEPENDENCY_UNAVAILABLE,
        ),
        cell(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::TypeScript,
            CellCapability::UnusedDependency,
            None,
            UNUSED_DEPENDENCY_UNAVAILABLE,
        ),
        cell(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::Elisp,
            CellCapability::UnusedDependency,
            None,
            UNUSED_DEPENDENCY_UNAVAILABLE,
        ),
        cell(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::Rust,
            CellCapability::UnreferencedExportedSymbol,
            Some(rust_unreferenced_symbols),
            SEMANTIC_UNAVAILABLE,
        ),
        cell(
            SignalFamily::UnusedDependenciesAndSymbols,
            Language::TypeScript,
            CellCapability::UnreferencedExportedSymbol,
            Some(typescript_unreferenced_symbols),
            SEMANTIC_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ClonesAndRepeatedTestShapes,
            Language::Rust,
            CellCapability::Default,
            Some(rust_clones),
            CLONE_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ClonesAndRepeatedTestShapes,
            Language::TypeScript,
            CellCapability::Default,
            Some(typescript_clones),
            CLONE_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ClonesAndRepeatedTestShapes,
            Language::Elisp,
            CellCapability::Default,
            Some(elisp_clones),
            CLONE_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ConversionAndErrorMapping,
            Language::Rust,
            CellCapability::Default,
            Some(rust_conversions),
            CONVERSION_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ConversionAndErrorMapping,
            Language::TypeScript,
            CellCapability::Default,
            Some(typescript_conversions),
            CONVERSION_UNAVAILABLE,
        ),
        cell(
            SignalFamily::ChurnAndCochange,
            Language::Repository,
            CellCapability::Default,
            Some(super::history::repository),
            HISTORY_UNAVAILABLE,
        ),
        cell(
            SignalFamily::AdapterPaths,
            Language::Repository,
            CellCapability::Default,
            Some(super::adapters::collect),
            ADAPTER_UNAVAILABLE,
        ),
    ];
    &CATALOG
}

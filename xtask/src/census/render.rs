use std::fmt::Write as _;

use super::{CellState, CensusReport, SignalSection};

/// Compact stable text for manual audit use. JSON serialization carries the same data model.
pub fn render_human(report: &CensusReport) -> String {
    let mut out = String::new();
    for section in &report.sections {
        writeln!(out, "{}: {}", signal_name(section), state_name(section)).unwrap();
        for cell in &section.cells {
            writeln!(
                out,
                "  {} / {} [{}; {}] — {}",
                signal_name_cell(cell.signal),
                language_name(cell.language),
                evidence_name(cell.collector.evidence_method),
                cell.collector.identity,
                state_detail(&cell.state),
            )
            .unwrap();
            writeln!(out, "    limitation: {}", cell.collector.limitation).unwrap();
            if let CellState::Candidates { candidates } = &cell.state {
                for candidate in candidates {
                    writeln!(
                        out,
                        "    candidate {}: {}",
                        candidate.identity, candidate.summary
                    )
                    .unwrap();
                    for path in &candidate.paths {
                        writeln!(out, "      {path}").unwrap();
                    }
                }
            }
        }
    }
    out
}

fn signal_name(section: &SignalSection) -> &'static str {
    signal_name_cell(section.signal)
}

fn signal_name_cell(signal: super::SignalFamily) -> &'static str {
    match signal {
        super::SignalFamily::DependencyStructure => "dependency structure",
        super::SignalFamily::ExportedSymbolReferences => "exported-symbol references",
        super::SignalFamily::ClonesAndRepeatedTestShapes => "clones and repeated test shapes",
        super::SignalFamily::ConversionAndErrorMapping => "conversion and error mapping",
        super::SignalFamily::UnusedDependenciesAndSymbols => "unused dependencies and symbols",
        super::SignalFamily::ChurnAndCochange => "churn and co-change",
        super::SignalFamily::AdapterPaths => "SQLite/PostgreSQL adapter paths",
    }
}

fn state_name(section: &SignalSection) -> &'static str {
    match section.state {
        super::orchestrate::SectionState::Clean => "clean",
        super::orchestrate::SectionState::Candidates => "candidates",
        super::orchestrate::SectionState::Unavailable => "unavailable",
        super::orchestrate::SectionState::Failed => "failed",
    }
}

fn language_name(language: super::Language) -> &'static str {
    match language {
        super::Language::Rust => "Rust",
        super::Language::TypeScript => "TypeScript",
        super::Language::Elisp => "Elisp",
        super::Language::Repository => "repository-wide",
    }
}

fn evidence_name(method: super::EvidenceMethod) -> &'static str {
    match method {
        super::EvidenceMethod::Semantic => "semantic",
        super::EvidenceMethod::Structural => "structural",
        super::EvidenceMethod::Heuristic => "heuristic",
    }
}

fn state_detail(state: &CellState) -> String {
    match state {
        CellState::Clean => "clean".into(),
        CellState::Candidates { candidates } => format!("{} candidate(s)", candidates.len()),
        CellState::Unavailable { capability } => format!("unavailable: {capability}"),
        CellState::Failed { error } => format!("failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::CollectorMetadata;
    use super::*;
    use crate::census::{CellReport, CensusReport, EvidenceMethod, Language, SignalFamily};

    #[test]
    fn human_rendering_is_deterministic_and_surfaces_unavailable_capability() {
        let report = CensusReport::from_cells(vec![CellReport {
            signal: SignalFamily::DependencyStructure,
            language: Language::Rust,
            collector: CollectorMetadata {
                identity: "fixture".into(),
                version: None,
                evidence_method: EvidenceMethod::Structural,
                limitation: "no parser".into(),
            },
            state: CellState::Unavailable {
                capability: "rust analyzer".into(),
            },
        }]);
        let first = render_human(&report);
        assert_eq!(first, render_human(&report));
        assert!(first.contains("unavailable: rust analyzer"));
        assert!(first.contains("[structural; fixture]"));
    }
}

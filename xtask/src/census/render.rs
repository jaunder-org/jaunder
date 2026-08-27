//! Deterministic human rendering for the census report.
//!
//! Rendering exposes each cell's independently keyed capability, provenance,
//! state, and bounded evidence. It reports unavailable and failed collection
//! truthfully and never upgrades either state to a clean result.

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
                "  {} / {} / {} [{}; {}{}] — {}",
                signal_name_cell(cell.signal),
                language_name(cell.language),
                capability_name(cell.capability),
                evidence_name(cell.collector.evidence_method),
                cell.collector.identity,
                cell.collector
                    .version
                    .as_deref()
                    .map(|version| format!("; version {version}"))
                    .unwrap_or_default(),
                state_detail(&cell.state),
            )
            .unwrap();
            writeln!(out, "    limitation: {}", cell.collector.limitation).unwrap();
            if let CellState::Candidates { candidates, .. } = &cell.state {
                for candidate in candidates {
                    writeln!(
                        out,
                        "    candidate {}: {}",
                        candidate.identity, candidate.summary
                    )
                    .unwrap();
                    if candidate.total_paths > candidate.paths.len() {
                        writeln!(
                            out,
                            "      paths: showing {} / {}",
                            candidate.paths.len(),
                            candidate.total_paths
                        )
                        .unwrap();
                    }
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
    language.display_name()
}

fn capability_name(capability: super::CellCapability) -> &'static str {
    match capability {
        super::CellCapability::Default => "default",
        super::CellCapability::UnusedDependency => "unused-dependency",
        super::CellCapability::UnreferencedExportedSymbol => "unreferenced-exported-symbol",
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
        CellState::Candidates {
            candidates,
            total_candidates,
        } => {
            let truncated = total_candidates.saturating_sub(candidates.len());
            if truncated == 0 {
                format!("{total_candidates} candidate(s)")
            } else {
                format!(
                    "{total_candidates} candidate(s); showing {} ({} truncated)",
                    candidates.len(),
                    truncated
                )
            }
        }
        CellState::Unavailable { reason } => format!("unavailable: {reason}"),
        CellState::Failed { error } => format!("failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        Candidate, CollectorMetadata, MAX_CANDIDATES_PER_CELL, MAX_PATHS_PER_CANDIDATE,
    };
    use super::*;
    use crate::census::{CellReport, CensusReport, EvidenceMethod, Language, SignalFamily};

    #[test]
    fn human_rendering_is_deterministic_and_surfaces_unavailable_capability() {
        let report = CensusReport::from_cells(vec![CellReport {
            signal: SignalFamily::DependencyStructure,
            language: Language::Rust,
            capability: super::super::CellCapability::Default,
            collector: CollectorMetadata {
                identity: "fixture".into(),
                version: Some("1.2.3".into()),
                evidence_method: EvidenceMethod::Structural,
                limitation: "no parser".into(),
            },
            state: CellState::Unavailable {
                reason: "rust analyzer".into(),
            },
        }]);
        let first = render_human(&report);
        assert_eq!(first, render_human(&report));
        assert!(first.contains("unavailable: rust analyzer"));
        assert!(first.contains("[structural; fixture; version 1.2.3]"));
        let json = serde_json::to_string(&report).expect("serializes report");
        assert!(json.contains("\"version\":\"1.2.3\""));
    }

    #[test]
    fn human_rendering_exposes_candidate_truncation() {
        let candidates = (0..(MAX_CANDIDATES_PER_CELL + 1))
            .map(|index| Candidate {
                identity: format!("candidate-{index:03}"),
                summary: "found".into(),
                paths: (0..(MAX_PATHS_PER_CANDIDATE + 1))
                    .map(|path| format!("web/src/{path}.rs"))
                    .collect(),
                total_paths: MAX_PATHS_PER_CANDIDATE + 1,
            })
            .collect();
        let report = CensusReport::from_cells(vec![CellReport::candidates(
            SignalFamily::DependencyStructure,
            Language::Rust,
            CollectorMetadata {
                identity: "fixture".into(),
                version: None,
                evidence_method: EvidenceMethod::Structural,
                limitation: "fixture limitation".into(),
            },
            candidates,
        )]);
        let rendered = render_human(&report);
        assert!(rendered.contains(&format!(
            "{} candidate(s); showing {} (1 truncated)",
            MAX_CANDIDATES_PER_CELL + 1,
            MAX_CANDIDATES_PER_CELL
        )));
        assert!(rendered.contains(&format!(
            "paths: showing {} / {}",
            MAX_PATHS_PER_CANDIDATE,
            MAX_PATHS_PER_CANDIDATE + 1
        )));
    }
}

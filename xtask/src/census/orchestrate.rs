//! Census cell orchestration and stable report aggregation.
//!
//! This module owns the seam between snapshot construction and independently
//! registered collectors. It validates each result, retains partial reports, and
//! preserves unavailable and failed states instead of treating absent evidence as
//! clean.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use serde::Serialize;

use super::{CellReport, CellSpec, CellState, Language, SignalFamily, SourceSnapshot};

/// Command-scoped inputs and memoized semantic evidence shared by census collectors.
///
/// The snapshot and cache exist only for one `collect` call. Semantic collectors
/// populate both projections for a language together, so exported-reference and
/// unreferenced-symbol cells cannot independently start an analyzer session.
pub struct CollectorContext {
    pub repo_root: std::path::PathBuf,
    pub snapshot: SourceSnapshot,
    pub(crate) semantic_reports: Mutex<BTreeMap<Language, (CellReport, CellReport)>>,
}

impl CollectorContext {
    fn from_repo(repo_root: &Path) -> Result<Self> {
        let mut context = Self {
            repo_root: repo_root.to_path_buf(),
            snapshot: SourceSnapshot::default(),
            semantic_reports: Mutex::default(),
        };
        context.snapshot = SourceSnapshot::from_tracked(&context.repo_root)?;
        Ok(context)
    }
}

/// One deterministic signal-family summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignalSection {
    pub signal: SignalFamily,
    pub state: SectionState,
    pub cells: Vec<CellReport>,
}

/// Aggregation preserves unavailable and failed cells rather than collapsing them into clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SectionState {
    Clean,
    Candidates,
    Unavailable,
    Failed,
}

/// The structured payload carried by the xtask result envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CensusReport {
    pub sections: Vec<SignalSection>,
}

impl CensusReport {
    pub fn from_cells(mut cells: Vec<CellReport>) -> Self {
        cells.sort_by_key(|cell| (cell.signal, cell.language, cell.capability));
        for cell in &mut cells {
            cell.normalize();
        }
        let sections = SignalFamily::ALL
            .into_iter()
            .map(|signal| {
                let cells: Vec<_> = cells
                    .iter()
                    .filter(|cell| cell.signal == signal)
                    .cloned()
                    .collect();
                SignalSection {
                    signal,
                    state: aggregate(&cells),
                    cells,
                }
            })
            .collect();
        Self { sections }
    }

    pub fn has_failed_cells(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|section| &section.cells)
            .any(CellReport::has_failed)
    }

    pub fn cell_count(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.cells.len())
            .sum()
    }
}

impl SignalFamily {
    pub const ALL: [Self; 7] = [
        Self::DependencyStructure,
        Self::ExportedSymbolReferences,
        Self::ClonesAndRepeatedTestShapes,
        Self::ConversionAndErrorMapping,
        Self::UnusedDependenciesAndSymbols,
        Self::ChurnAndCochange,
        Self::AdapterPaths,
    ];
}

fn aggregate(cells: &[CellReport]) -> SectionState {
    if cells.iter().any(CellReport::has_failed) {
        SectionState::Failed
    } else if cells
        .iter()
        .any(|cell| matches!(cell.state, CellState::Candidates { .. }))
    {
        SectionState::Candidates
    } else if cells
        .iter()
        .any(|cell| matches!(cell.state, CellState::Unavailable { .. }))
    {
        SectionState::Unavailable
    } else {
        SectionState::Clean
    }
}

/// Collect the catalogued cells over a fresh Git-tracked working-tree snapshot.
pub fn collect(repo_root: &Path, catalog: &[CellSpec]) -> Result<CensusReport> {
    let context = CollectorContext::from_repo(repo_root)?;
    let mut cells = catalog
        .iter()
        .map(|spec| unavailable(&context.snapshot, spec))
        .collect::<Vec<_>>();
    for spec in catalog {
        let Some(collect) = spec.collect else {
            continue;
        };
        let report = validate(collect(&context));
        if report.signal != spec.signal
            || report.language != spec.language
            || report.capability != spec.capability
        {
            replace_cell(
                &mut cells,
                spec,
                CellReport {
                    signal: spec.signal,
                    language: spec.language,
                    capability: spec.capability,
                    collector: report.collector,
                    state: CellState::Failed {
                        error: "collector returned a report for a different cell".into(),
                    },
                },
            );
            continue;
        }
        replace_cell(&mut cells, spec, report);
    }
    Ok(CensusReport::from_cells(cells))
}

fn replace_cell(cells: &mut Vec<CellReport>, spec: &CellSpec, report: CellReport) {
    cells.retain(|cell| {
        !(cell.signal == spec.signal
            && cell.language == spec.language
            && cell.capability == spec.capability)
    });
    cells.push(report);
}

fn validate(mut report: CellReport) -> CellReport {
    let invalid_metadata = report.collector.identity.trim().is_empty()
        || report.collector.limitation.trim().is_empty();
    let invalid_candidates = match &report.state {
        CellState::Candidates {
            candidates,
            total_candidates,
        } => {
            candidates.is_empty()
                || *total_candidates < candidates.len()
                || candidates.iter().any(|candidate| {
                    candidate.identity.trim().is_empty()
                        || candidate.summary.trim().is_empty()
                        || candidate.paths.is_empty()
                        || candidate.total_paths < candidate.paths.len()
                        || candidate.paths.iter().any(|path| {
                            path.is_empty()
                                || Path::new(path).is_absolute()
                                || path.split('/').any(|part| part == "..")
                        })
                })
        }
        _ => false,
    };
    if invalid_metadata || invalid_candidates {
        report.state = CellState::Failed {
            error: "collector returned malformed metadata or candidates".into(),
        };
    }
    report
}

fn unavailable(snapshot: &SourceSnapshot, spec: &CellSpec) -> CellReport {
    let reason = if has_source_inputs(snapshot, spec.language) {
        spec.unavailable_reason.to_owned()
    } else {
        format!(
            "{}; no tracked {:?} source inputs",
            spec.unavailable_reason, spec.language
        )
    };
    CellReport::unavailable(spec.signal, spec.language, reason).with_capability(spec.capability)
}

fn has_source_inputs(snapshot: &SourceSnapshot, language: Language) -> bool {
    snapshot
        .files
        .iter()
        .any(|file| language == Language::Repository || language.matches_path(&file.path))
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        Candidate, CellCapability, CollectorMetadata, MAX_CANDIDATES_PER_CELL,
        MAX_PATHS_PER_CANDIDATE,
    };
    use super::*;
    use crate::census::EvidenceMethod;

    fn report(signal: SignalFamily, language: Language, state: CellState) -> CellReport {
        CellReport {
            signal,
            language,
            capability: CellCapability::Default,
            collector: CollectorMetadata {
                identity: "fixture".into(),
                version: Some("1".into()),
                evidence_method: EvidenceMethod::Structural,
                limitation: "fixture limitation".into(),
            },
            state,
        }
    }

    #[test]
    fn aggregation_keeps_unavailable_distinct_from_clean() {
        let census = CensusReport::from_cells(vec![
            report(
                SignalFamily::DependencyStructure,
                Language::Rust,
                CellState::Clean,
            ),
            report(
                SignalFamily::DependencyStructure,
                Language::TypeScript,
                CellState::Unavailable {
                    reason: "semantic analyzer".into(),
                },
            ),
        ]);
        assert_eq!(census.sections[0].state, SectionState::Unavailable);
    }

    #[test]
    fn catalog_keeps_unused_dependency_cells_beside_unreferenced_symbol_cells() {
        let cells = super::super::registry::catalog();
        for language in [Language::Rust, Language::TypeScript, Language::Elisp] {
            assert!(cells.iter().any(|cell| {
                cell.signal == SignalFamily::UnusedDependenciesAndSymbols
                    && cell.language == language
                    && cell.capability == CellCapability::UnusedDependency
                    && cell.collect.is_none()
            }));
        }
        for language in [Language::Rust, Language::TypeScript] {
            assert!(cells.iter().any(|cell| {
                cell.signal == SignalFamily::UnusedDependenciesAndSymbols
                    && cell.language == language
                    && cell.capability == CellCapability::UnreferencedExportedSymbol
                    && cell.collect.is_some()
            }));
        }
    }

    #[test]
    fn failed_cell_preserves_completed_cells_and_fails_report() {
        let candidate = Candidate {
            identity: "candidate-b".into(),
            summary: "found".into(),
            total_paths: 2,
            paths: vec!["web/src/b.rs".into(), "web/src/a.rs".into()],
        };
        let census = CensusReport::from_cells(vec![
            report(
                SignalFamily::DependencyStructure,
                Language::Rust,
                CellState::Candidates {
                    candidates: vec![candidate],
                    total_candidates: 1,
                },
            ),
            report(
                SignalFamily::DependencyStructure,
                Language::TypeScript,
                CellState::Failed {
                    error: "malformed output".into(),
                },
            ),
        ]);
        assert!(census.has_failed_cells());
        assert_eq!(census.sections[0].state, SectionState::Failed);
        assert_eq!(census.sections[0].cells.len(), 2);
        let CellState::Candidates { candidates, .. } = &census.sections[0].cells[0].state else {
            panic!("completed candidate cell was discarded");
        };
        assert_eq!(candidates[0].paths, ["web/src/a.rs", "web/src/b.rs"]);
    }

    #[test]
    fn json_serialization_is_deterministic_and_includes_each_section() {
        let census = CensusReport::from_cells(vec![report(
            SignalFamily::DependencyStructure,
            Language::Rust,
            CellState::Clean,
        )]);
        let first = serde_json::to_string(&census).unwrap();
        assert_eq!(first, serde_json::to_string(&census).unwrap());
        assert_eq!(census.sections.len(), SignalFamily::ALL.len());
        assert!(first.contains("\"state\":\"clean\""));
    }

    #[test]
    fn candidate_cap_is_deterministic_and_keeps_total_in_json() {
        let candidates = (0..(MAX_CANDIDATES_PER_CELL + 2))
            .rev()
            .map(|index| Candidate {
                identity: format!("candidate-{index:03}"),
                summary: "found".into(),
                total_paths: MAX_PATHS_PER_CANDIDATE + 1,
                paths: (0..(MAX_PATHS_PER_CANDIDATE + 1))
                    .map(|path| format!("web/src/{index}-{path}.rs"))
                    .collect(),
            })
            .collect();
        let census = CensusReport::from_cells(vec![CellReport::candidates(
            SignalFamily::DependencyStructure,
            Language::Rust,
            CollectorMetadata {
                identity: "fixture".into(),
                version: Some("1".into()),
                evidence_method: EvidenceMethod::Structural,
                limitation: "fixture limitation".into(),
            },
            candidates,
        )]);
        let state = &census.sections[0].cells[0].state;
        let CellState::Candidates {
            candidates,
            total_candidates,
        } = state
        else {
            panic!("candidate cell became non-candidate");
        };
        assert_eq!(candidates.len(), MAX_CANDIDATES_PER_CELL);
        assert_eq!(*total_candidates, MAX_CANDIDATES_PER_CELL + 2);
        assert_eq!(candidates[0].identity, "candidate-000");
        assert_eq!(candidates[0].paths.len(), MAX_PATHS_PER_CANDIDATE);
        assert_eq!(candidates[0].total_paths, MAX_PATHS_PER_CANDIDATE + 1);
        let serialized = serde_json::to_string(&census).expect("serializes capped report");
        assert!(serialized.contains(&format!(
            "\"total_candidates\":{}",
            MAX_CANDIDATES_PER_CELL + 2
        )));
        assert!(serialized.contains(&format!("\"total_paths\":{}", MAX_PATHS_PER_CANDIDATE + 1)));
        assert_eq!(serialized, serde_json::to_string(&census).unwrap());
    }
}

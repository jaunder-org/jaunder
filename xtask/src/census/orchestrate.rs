use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use super::{CellReport, CellState, CollectorSpec, Language, SignalFamily, SourceSnapshot};

/// Immutable input shared by every collector for one command invocation.
pub struct CollectorContext {
    pub repo_root: std::path::PathBuf,
    pub snapshot: SourceSnapshot,
}

impl CollectorContext {
    fn from_repo(repo_root: &Path) -> Result<Self> {
        let mut context = Self {
            repo_root: repo_root.to_path_buf(),
            snapshot: SourceSnapshot::default(),
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
        cells.sort_by_key(|cell| (cell.signal, cell.language));
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

/// Collect the registered cells over a fresh Git-tracked working-tree snapshot.
pub fn collect(repo_root: &Path, specs: &[CollectorSpec]) -> Result<CensusReport> {
    let context = CollectorContext::from_repo(repo_root)?;
    let mut cells = required_cells(&context.snapshot);
    for spec in specs {
        let report = validate((spec.collect)(&context));
        if report.signal != spec.signal || report.language != spec.language {
            cells.retain(|cell| !(cell.signal == spec.signal && cell.language == spec.language));
            cells.push(CellReport {
                signal: spec.signal,
                language: spec.language,
                collector: report.collector,
                state: CellState::Failed {
                    error: "collector returned a report for a different cell".into(),
                },
            });
            continue;
        }
        cells.retain(|cell| !(cell.signal == spec.signal && cell.language == spec.language));
        cells.push(report);
    }
    Ok(CensusReport::from_cells(cells))
}

fn validate(mut report: CellReport) -> CellReport {
    let invalid_metadata = report.collector.identity.trim().is_empty()
        || report.collector.limitation.trim().is_empty();
    let invalid_candidates = match &report.state {
        CellState::Candidates { candidates } => {
            candidates.is_empty()
                || candidates.iter().any(|candidate| {
                    candidate.identity.trim().is_empty()
                        || candidate.summary.trim().is_empty()
                        || candidate.paths.is_empty()
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

fn required_cells(snapshot: &SourceSnapshot) -> Vec<CellReport> {
    let mut cells = Vec::new();
    for language in [Language::Rust, Language::TypeScript, Language::Elisp] {
        cells.push(unavailable(
            snapshot,
            SignalFamily::DependencyStructure,
            language,
            "dependency collector is not available",
        ));
        cells.push(unavailable(
            snapshot,
            SignalFamily::ClonesAndRepeatedTestShapes,
            language,
            "structural clone collector is not available",
        ));
        cells.push(unavailable(
            snapshot,
            SignalFamily::UnusedDependenciesAndSymbols,
            language,
            "unused-dependency or symbol analyzer is not available",
        ));
        cells.push(unavailable(
            snapshot,
            SignalFamily::ChurnAndCochange,
            language,
            "history collector is not available",
        ));
    }
    for language in [Language::Rust, Language::TypeScript] {
        cells.push(unavailable(
            snapshot,
            SignalFamily::ExportedSymbolReferences,
            language,
            "semantic analyzer is not available",
        ));
        cells.push(unavailable(
            snapshot,
            SignalFamily::ConversionAndErrorMapping,
            language,
            "structural conversion collector is not available",
        ));
    }
    cells.push(unavailable(
        snapshot,
        SignalFamily::AdapterPaths,
        Language::Repository,
        "SQLite/PostgreSQL adapter-path collector is not available",
    ));
    cells
}

fn unavailable(
    snapshot: &SourceSnapshot,
    signal: SignalFamily,
    language: Language,
    capability: &str,
) -> CellReport {
    let capability = if has_source_inputs(snapshot, language) {
        capability.to_owned()
    } else {
        format!("{capability}; no tracked {language:?} source inputs")
    };
    CellReport::unavailable(signal, language, capability)
}

fn has_source_inputs(snapshot: &SourceSnapshot, language: Language) -> bool {
    snapshot.files.iter().any(|file| match language {
        Language::Rust => file.path.ends_with(".rs"),
        Language::TypeScript => {
            file.path.ends_with(".ts")
                || file.path.ends_with(".tsx")
                || file.path.ends_with(".js")
                || file.path.ends_with(".jsx")
        }
        Language::Elisp => file.path.ends_with(".el"),
        Language::Repository => true,
    })
}

#[cfg(test)]
mod tests {
    use super::super::model::{Candidate, CollectorMetadata};
    use super::*;
    use crate::census::EvidenceMethod;

    fn report(signal: SignalFamily, language: Language, state: CellState) -> CellReport {
        CellReport {
            signal,
            language,
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
                    capability: "semantic analyzer".into(),
                },
            ),
        ]);
        assert_eq!(census.sections[0].state, SectionState::Unavailable);
    }

    #[test]
    fn failed_cell_preserves_completed_cells_and_fails_report() {
        let candidate = Candidate {
            identity: "candidate-b".into(),
            summary: "found".into(),
            paths: vec!["web/src/b.rs".into(), "web/src/a.rs".into()],
        };
        let census = CensusReport::from_cells(vec![
            report(
                SignalFamily::DependencyStructure,
                Language::Rust,
                CellState::Candidates {
                    candidates: vec![candidate],
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
        let CellState::Candidates { candidates } = &census.sections[0].cells[0].state else {
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
}

use serde::Serialize;

/// The owned-language surface a collector describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    Rust,
    TypeScript,
    Elisp,
    Repository,
}

/// A stable report section; ordering follows the audit workflow rather than registration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalFamily {
    DependencyStructure,
    ExportedSymbolReferences,
    ClonesAndRepeatedTestShapes,
    ConversionAndErrorMapping,
    UnusedDependenciesAndSymbols,
    ChurnAndCochange,
    AdapterPaths,
}

/// How a collector established its evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceMethod {
    Semantic,
    Structural,
    Heuristic,
}

/// Provenance that keeps candidates useful without overstating their certainty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollectorMetadata {
    pub identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub evidence_method: EvidenceMethod,
    pub limitation: String,
}

/// A repository-relative candidate emitted by a completed collector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub identity: String,
    pub summary: String,
    pub paths: Vec<String>,
    pub total_paths: usize,
}

impl Candidate {
    pub fn new(identity: String, summary: String, paths: Vec<String>) -> Self {
        let total_paths = paths.len();
        Self {
            identity,
            summary,
            paths,
            total_paths,
        }
    }
}

/// The maximum number of candidates retained per cell in both human and JSON reports.
pub const MAX_CANDIDATES_PER_CELL: usize = 10;

/// The maximum number of paths retained per candidate in both human and JSON reports.
pub const MAX_PATHS_PER_CANDIDATE: usize = 10;

/// The only possible result for a required language/signal cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum CellState {
    Clean,
    Candidates {
        candidates: Vec<Candidate>,
        total_candidates: usize,
    },
    Unavailable {
        capability: String,
    },
    Failed {
        error: String,
    },
}

/// One collector result. The orchestration layer owns aggregation and exit policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellReport {
    pub signal: SignalFamily,
    pub language: Language,
    pub collector: CollectorMetadata,
    #[serde(flatten)]
    pub state: CellState,
}

impl CellReport {
    pub fn unavailable(
        signal: SignalFamily,
        language: Language,
        capability: impl Into<String>,
    ) -> Self {
        let capability = capability.into();
        Self {
            signal,
            language,
            collector: CollectorMetadata {
                identity: "census-contract".into(),
                version: None,
                evidence_method: EvidenceMethod::Heuristic,
                limitation: format!("{capability}; no collector supplied a result"),
            },
            state: CellState::Unavailable { capability },
        }
    }

    pub fn unavailable_with_collector(
        signal: SignalFamily,
        language: Language,
        collector: CollectorMetadata,
        capability: impl Into<String>,
    ) -> Self {
        Self {
            signal,
            language,
            collector,
            state: CellState::Unavailable {
                capability: capability.into(),
            },
        }
    }

    pub fn candidates(
        signal: SignalFamily,
        language: Language,
        collector: CollectorMetadata,
        candidates: Vec<Candidate>,
    ) -> Self {
        let total_candidates = candidates.len();
        Self {
            signal,
            language,
            collector,
            state: if candidates.is_empty() {
                CellState::Clean
            } else {
                CellState::Candidates {
                    candidates,
                    total_candidates,
                }
            },
        }
    }

    pub fn has_failed(&self) -> bool {
        matches!(self.state, CellState::Failed { .. })
    }

    pub(crate) fn normalize(&mut self) {
        if let CellState::Candidates {
            candidates,
            total_candidates,
        } = &mut self.state
        {
            for candidate in candidates.iter_mut() {
                candidate.paths.sort();
                candidate.paths.dedup();
                candidate.total_paths = candidate.paths.len();
                candidate.paths.truncate(MAX_PATHS_PER_CANDIDATE);
            }
            candidates.sort_by(|left, right| {
                left.identity
                    .cmp(&right.identity)
                    .then_with(|| left.summary.cmp(&right.summary))
                    .then_with(|| left.paths.cmp(&right.paths))
            });
            *total_candidates = candidates.len();
            candidates.truncate(MAX_CANDIDATES_PER_CELL);
        }
    }
}

/// A collector registration. Collectors return data only; rendering and command failure remain
/// centralized so a failed collector cannot discard reports already collected.
pub struct CollectorSpec {
    pub signal: SignalFamily,
    pub language: Language,
    pub collect: fn(&crate::census::CollectorContext) -> CellReport,
}

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
}

/// The only possible result for a required language/signal cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum CellState {
    Clean,
    Candidates { candidates: Vec<Candidate> },
    Unavailable { capability: String },
    Failed { error: String },
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

    pub fn has_failed(&self) -> bool {
        matches!(self.state, CellState::Failed { .. })
    }

    pub(crate) fn normalize(&mut self) {
        if let CellState::Candidates { candidates } = &mut self.state {
            for candidate in candidates.iter_mut() {
                candidate.paths.sort();
                candidate.paths.dedup();
            }
            candidates.sort_by(|left, right| left.identity.cmp(&right.identity));
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

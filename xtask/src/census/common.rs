//! Shared support for structural census collectors.
//!
//! This module owns collector-result construction, source selection, and parser
//! validation shared by otherwise independent dependency, clone, and conversion
//! collectors. It does not choose report ordering or command failure policy.

use super::model::{Candidate, CellCapability, CollectorMetadata};
use super::source::language_for_path;
use super::{CellReport, CellState, CollectorContext, EvidenceMethod, Language, SignalFamily};

pub(crate) const STRUCTURAL_VERSION: &str = "1";

pub(crate) fn structural(
    signal: SignalFamily,
    language: Language,
    limitation: &str,
    candidates: Vec<Candidate>,
) -> CellReport {
    CellReport::candidates(
        signal,
        language,
        CollectorMetadata {
            identity: format!("census-{}-structural", language.slug()),
            version: Some(STRUCTURAL_VERSION.into()),
            evidence_method: EvidenceMethod::Structural,
            limitation: limitation.into(),
        },
        candidates,
    )
}

pub(crate) fn failed(
    signal: SignalFamily,
    language: Language,
    identity: &str,
    error: String,
) -> CellReport {
    CellReport {
        signal,
        language,
        capability: CellCapability::Default,
        collector: CollectorMetadata {
            identity: identity.into(),
            version: Some(STRUCTURAL_VERSION.into()),
            evidence_method: EvidenceMethod::Structural,
            limitation: "source could not be parsed structurally".into(),
        },
        state: CellState::Failed { error },
    }
}

pub(crate) fn files(
    context: &CollectorContext,
    language: Language,
) -> impl Iterator<Item = (&str, &str)> {
    context.snapshot.files.iter().filter_map(move |file| {
        (language_for_path(&file.path) == Some(language))
            .then_some((file.path.as_str(), file.content.as_str()))
    })
}

pub(crate) fn balanced_elisp(source: &str) -> bool {
    source.chars().fold(0i32, |depth, character| {
        depth + i32::from(character == '(') - i32::from(character == ')')
    }) == 0
}

#[cfg(test)]
pub(crate) fn context(files: &[(&str, &str)]) -> CollectorContext {
    use super::SourceSnapshot;
    use super::snapshot::SourceFile;

    CollectorContext {
        repo_root: ".".into(),
        snapshot: SourceSnapshot {
            files: files
                .iter()
                .map(|(path, content)| SourceFile {
                    path: (*path).into(),
                    content: (*content).into(),
                })
                .collect(),
        },
        semantic_reports: Default::default(),
    }
}

//! Storage-adapter path correspondence collector.
//!
//! This module compares approved SQLite and PostgreSQL path inventories through
//! the shared snapshot seam. It emits heuristic correspondence candidates only,
//! never behavioral-equivalence claims; missing paths simply remain unmatched.

use std::collections::BTreeSet;

use super::model::{Candidate, CollectorMetadata};
use super::{CellReport, CollectorContext, EvidenceMethod, Language, SignalFamily};

const ADAPTER_VERSION: &str = "1";
const SQLITE_ROOT: &str = "storage/src/sqlite/";
const POSTGRES_ROOT: &str = "storage/src/postgres/";

pub(crate) fn collect(context: &CollectorContext) -> CellReport {
    let sqlite = context
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.path.strip_prefix(SQLITE_ROOT).map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let postgres = context
        .snapshot
        .files
        .iter()
        .filter_map(|file| file.path.strip_prefix(POSTGRES_ROOT).map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for relative in sqlite.intersection(&postgres) {
        candidates.push(Candidate {
            identity: format!("adapter-pair:{relative}"),
            summary: format!("SQLite/PostgreSQL path correspondence candidate for {relative}; not a semantic-equivalence claim"),
            total_paths: 2,
            paths: vec![format!("{SQLITE_ROOT}{relative}"), format!("{POSTGRES_ROOT}{relative}")],
        });
    }
    for relative in sqlite.difference(&postgres) {
        candidates.push(Candidate {
            identity: format!("adapter-unmatched:sqlite:{relative}"),
            summary: format!(
                "SQLite adapter path has no PostgreSQL path correspondence candidate for {relative}"
            ),
            total_paths: 1,
            paths: vec![format!("{SQLITE_ROOT}{relative}")],
        });
    }
    for relative in postgres.difference(&sqlite) {
        candidates.push(Candidate {
            identity: format!("adapter-unmatched:postgres:{relative}"),
            summary: format!(
                "PostgreSQL adapter path has no SQLite path correspondence candidate for {relative}"
            ),
            total_paths: 1,
            paths: vec![format!("{POSTGRES_ROOT}{relative}")],
        });
    }
    CellReport::candidates(
        SignalFamily::AdapterPaths,
        Language::Repository,
        CollectorMetadata {
            identity: "census-storage-adapter-paths".into(),
            version: Some(ADAPTER_VERSION.into()),
            evidence_method: EvidenceMethod::Heuristic,
            limitation: "Matches only approved tracked storage/src/sqlite and storage/src/postgres paths. Pairing is a review aid and does not establish behavioral, SQL, or semantic equivalence.".into(),
        },
        candidates,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::snapshot::SourceFile;
    use super::*;
    use crate::census::{CellState, SourceSnapshot};

    #[test]
    fn reports_paired_and_unmatched_adapter_paths_without_parity_claims() {
        let context = CollectorContext {
            repo_root: Default::default(),
            snapshot: SourceSnapshot {
                files: vec![
                    SourceFile {
                        path: "storage/src/sqlite/session.rs".into(),
                        content: String::new(),
                    },
                    SourceFile {
                        path: "storage/src/postgres/session.rs".into(),
                        content: String::new(),
                    },
                    SourceFile {
                        path: "storage/src/sqlite/backup.rs".into(),
                        content: String::new(),
                    },
                    SourceFile {
                        path: "storage/src/postgres/catalog.rs".into(),
                        content: String::new(),
                    },
                ],
            },
        };
        let report = collect(&context);
        assert_eq!(report.collector.evidence_method, EvidenceMethod::Heuristic);
        assert!(report.collector.limitation.contains("does not establish"));
        let CellState::Candidates { candidates, .. } = report.state else {
            panic!("adapter candidates expected")
        };
        let identities = candidates
            .into_iter()
            .map(|candidate| candidate.identity)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            identities,
            BTreeSet::from([
                "adapter-pair:session.rs".into(),
                "adapter-unmatched:sqlite:backup.rs".into(),
                "adapter-unmatched:postgres:catalog.rs".into(),
            ])
        );
    }

    #[test]
    fn reports_clean_without_storage_adapter_paths() {
        let report = collect(&CollectorContext {
            repo_root: Default::default(),
            snapshot: SourceSnapshot {
                files: vec![SourceFile {
                    path: "server/src/lib.rs".into(),
                    content: String::new(),
                }],
            },
        });

        assert!(matches!(report.state, CellState::Clean));
        assert_eq!(report.collector.evidence_method, EvidenceMethod::Heuristic);
    }
}

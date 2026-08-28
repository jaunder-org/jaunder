//! Git-history census collector for repository-wide churn and co-change.
//!
//! The collector walks full non-merge history reachable from `HEAD`, attributes
//! renames to approved current paths, and reports deterministic heuristic review
//! candidates. Repository collection includes every approved HEAD-tracked path,
//! not merely recognized language sources; optional language selection exists
//! only for focused tests. Git failures stay visible as failed cells.

use std::collections::{BTreeMap, BTreeSet};

use super::model::{Candidate, CellCapability, CollectorMetadata};
use super::snapshot::is_approved_path;
use super::source::language_for_path;
use super::{CellReport, CellState, CollectorContext, EvidenceMethod, Language, SignalFamily};
const HISTORY_VERSION: &str = "1";
const MINIMUM_OBSERVATIONS: usize = 2;

pub(crate) fn repository(context: &CollectorContext) -> CellReport {
    collect(context, Language::Repository)
}

#[cfg(test)]
fn rust(context: &CollectorContext) -> CellReport {
    collect(context, Language::Rust)
}

fn collect(context: &CollectorContext, language: Language) -> CellReport {
    let metadata = CollectorMetadata {
        identity: format!("census-{}-git-history", language_name(language)),
        version: Some(HISTORY_VERSION.into()),
        evidence_method: EvidenceMethod::Heuristic,
        limitation: "Counts full HEAD-reachable non-merge Git history with rename attribution; counts are heuristic maintenance signals, not causal evidence.".into(),
    };
    match history_facts(context, language) {
        Ok((churn, cochange)) => {
            let mut candidates = churn
                .into_iter()
                .filter(|(_, count)| *count >= MINIMUM_OBSERVATIONS)
                .map(|(path, count)| Candidate {
                    identity: format!("churn:{path}"),
                    summary: format!("changed in {count} non-merge commits"),
                    total_paths: 1,
                    paths: vec![path],
                })
                .collect::<Vec<_>>();
            candidates.extend(cochange.into_iter().filter_map(|((left, right), count)| {
                (count >= MINIMUM_OBSERVATIONS).then(|| Candidate {
                    identity: format!("cochange:{left}+{right}"),
                    summary: format!("co-changed in {count} non-merge commits"),
                    total_paths: 2,
                    paths: vec![left, right],
                })
            }));
            CellReport::candidates(
                SignalFamily::ChurnAndCochange,
                language,
                metadata,
                candidates,
            )
        }
        Err(error) => CellReport {
            signal: SignalFamily::ChurnAndCochange,
            language,
            capability: CellCapability::Default,
            collector: metadata,
            state: CellState::Failed { error },
        },
    }
}

type HistoryFacts = (BTreeMap<String, usize>, BTreeMap<(String, String), usize>);

/// Derive current-path churn and co-change facts by walking commits newest-first.
/// Mapping each historical rename back to its current destination ensures earlier edits retain
/// the identity users can open in today's checkout. HEAD selects the history surface:
/// working-tree deletions only affect source collectors and must not erase committed churn.
fn history_facts(context: &CollectorContext, language: Language) -> Result<HistoryFacts, String> {
    let current_paths = git_output(
        context,
        &["ls-tree", "--full-tree", "-r", "-z", "--name-only", "HEAD"],
    )?
    .split('\0')
    .filter(|path| is_approved_path(path))
    .filter(|path| !path.is_empty())
    .filter(|path| belongs_to_language(path, language))
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let commits = git_output(context, &["rev-list", "--no-merges", "HEAD"])?;
    let mut names = current_paths
        .iter()
        .map(|path| (path.clone(), path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut churn = BTreeMap::new();
    let mut cochange = BTreeMap::new();

    for commit in commits.lines().filter(|commit| !commit.is_empty()) {
        let output = git_output(
            context,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--name-status",
                "-z",
                "--find-renames=50%",
                "-r",
                commit,
            ],
        )?;
        let changed = current_paths_for_commit(&output, &mut names)?;
        let selected = changed
            .into_iter()
            .filter(|path| belongs_to_language(path, language))
            .collect::<Vec<_>>();
        for path in &selected {
            *churn.entry(path.clone()).or_insert(0) += 1;
        }
        for (index, left) in selected.iter().enumerate() {
            for right in selected.iter().skip(index + 1) {
                *cochange.entry((left.clone(), right.clone())).or_insert(0) += 1;
            }
        }
    }
    Ok((churn, cochange))
}

fn git_output(context: &CollectorContext, args: &[&str]) -> Result<String, String> {
    let output = crate::git::at(&context.repo_root)
        .args(args)
        .output()
        .map_err(|error| format!("running git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("git {} produced non-UTF-8 output: {error}", args.join(" ")))
}

fn current_paths_for_commit(
    output: &str,
    names: &mut BTreeMap<String, String>,
) -> Result<BTreeSet<String>, String> {
    let mut fields = output.split('\0').filter(|field| !field.is_empty());
    let mut changed = BTreeSet::new();
    while let Some(status) = fields.next() {
        let path = fields
            .next()
            .ok_or_else(|| format!("git name-status omitted a path after {status:?}"))?;
        if status.starts_with('R') {
            let destination = fields
                .next()
                .ok_or_else(|| format!("git rename status omitted destination after {status:?}"))?;
            if let Some(current) = names.get(destination).cloned() {
                names.insert(path.to_owned(), current.clone());
                changed.insert(current);
            }
        } else if let Some(current) = names.get(path) {
            changed.insert(current.clone());
        }
    }
    Ok(changed)
}

fn belongs_to_language(path: &str, language: Language) -> bool {
    language == Language::Repository || language_for_path(path) == Some(language)
}

fn language_name(language: Language) -> &'static str {
    language.slug()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::census::SourceSnapshot;

    fn git(root: &Path, args: &[&str]) {
        let output = crate::git::at(root).args(args).output().expect("runs git");
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit(root: &Path, message: &str) {
        git(root, &["add", "."]);
        git(root, &["commit", "-m", message]);
    }

    fn context(root: &Path) -> CollectorContext {
        CollectorContext {
            repo_root: root.to_path_buf(),
            snapshot: SourceSnapshot::from_tracked(root).expect("snapshot"),
            semantic_reports: Default::default(),
        }
    }

    #[test]
    fn attributes_churn_and_cochange_to_current_rename_and_excludes_merge_commit_only_changes() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let root = temporary.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "census@example.test"]);
        git(root, &["config", "user.name", "Census Fixture"]);
        fs::create_dir_all(root.join("server/src")).expect("source dir");
        fs::write(root.join("server/src/a.rs"), "fn a() {}\n").expect("a");
        fs::write(root.join("server/src/b.rs"), "fn b() {}\n").expect("b");
        commit(root, "initial source");
        fs::write(root.join("server/src/a.rs"), "fn a() { first(); }\n").expect("a change");
        fs::write(root.join("server/src/b.rs"), "fn b() { first(); }\n").expect("b change");
        commit(root, "cochange old name");
        git(root, &["mv", "server/src/a.rs", "server/src/hot.rs"]);
        commit(root, "rename source");
        fs::write(root.join("server/src/hot.rs"), "fn a() { second(); }\n").expect("hot change");
        fs::write(root.join("server/src/b.rs"), "fn b() { second(); }\n").expect("b change");
        commit(root, "cochange current name");
        git(root, &["checkout", "-b", "side"]);
        fs::write(root.join("server/src/side.rs"), "fn side() {}\n").expect("side");
        commit(root, "side source");
        git(root, &["checkout", "-"]);
        git(root, &["merge", "--no-ff", "--no-commit", "side"]);
        fs::write(
            root.join("server/src/merge_only.rs"),
            "fn merge_only() {}\n",
        )
        .expect("merge only");
        commit(root, "merge with unique file");

        let report = rust(&context(root));
        let CellState::Candidates { candidates, .. } = report.state else {
            panic!("history candidates expected")
        };
        let identities = candidates
            .into_iter()
            .map(|candidate| candidate.identity)
            .collect::<BTreeSet<_>>();
        assert!(identities.contains("churn:server/src/hot.rs"));
        assert!(identities.contains("cochange:server/src/b.rs+server/src/hot.rs"));
        assert!(!identities.contains("churn:server/src/merge_only.rs"));
    }

    #[test]
    fn reports_clean_when_history_is_below_the_observation_threshold() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let root = temporary.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "census@example.test"]);
        git(root, &["config", "user.name", "Census Fixture"]);
        fs::create_dir_all(root.join("server/src")).expect("source dir");
        fs::write(root.join("server/src/a.rs"), "fn a() {}\n").expect("source");
        commit(root, "initial source");

        assert!(matches!(rust(&context(root)).state, CellState::Clean));
    }

    #[test]
    fn uncommitted_deletion_does_not_remove_head_history_from_churn() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let root = temporary.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "census@example.test"]);
        git(root, &["config", "user.name", "Census Fixture"]);
        fs::create_dir_all(root.join("server/src")).expect("source dir");
        let path = root.join("server/src/a.rs");
        fs::write(&path, "fn a() {}\n").expect("initial source");
        commit(root, "initial source");
        fs::write(&path, "fn a() { changed(); }\n").expect("changed source");
        commit(root, "changed source");
        fs::remove_file(path).expect("uncommitted deletion");

        let report = rust(&context(root));
        let CellState::Candidates { candidates, .. } = report.state else {
            panic!("committed churn remains represented after a dirty deletion");
        };
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.identity == "churn:server/src/a.rs")
        );
    }

    #[test]
    fn repository_history_includes_approved_non_language_paths() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let root = temporary.path();
        git(root, &["init"]);
        git(root, &["config", "user.email", "census@example.test"]);
        git(root, &["config", "user.name", "Census Fixture"]);
        fs::create_dir_all(root.join("xtask")).expect("xtask dir");
        let path = root.join("xtask/fixture.txt");
        fs::write(&path, "<svg/>").expect("initial fixture");
        commit(root, "initial fixture");
        fs::write(&path, "<svg><!-- changed --></svg>").expect("changed fixture");
        commit(root, "changed fixture");

        let CellState::Candidates { candidates, .. } = repository(&context(root)).state else {
            panic!("approved non-language history is included");
        };
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.identity == "churn:xtask/fixture.txt")
        );
    }
    #[test]
    fn reports_git_failures_explicitly() {
        let temporary = tempfile::tempdir().expect("tempdir");
        let report = rust(&CollectorContext {
            repo_root: temporary.path().to_path_buf(),
            snapshot: SourceSnapshot::default(),
            semantic_reports: Default::default(),
        });
        assert!(matches!(report.state, CellState::Failed { .. }));
    }
}

use std::path::Path;

use anyhow::{Context, Result};

use super::files::{adr_link, parse_adr_dir};

/// The materialized view of the architecture. Every accepted ADR must be *cited*
/// here — `view_parity_problems` checks exactly that, and no more.
pub const VIEW: &str = "docs/ARCHITECTURE.md";

/// The `adr-view-parity` problems: every **accepted** ADR must be cited at least
/// once in `docs/ARCHITECTURE.md`. `superseded`, `deprecated` and `rejected` are
/// excluded — the view describes the architecture as it stands, not its history.
///
/// A citation is either a markdown link whose target is `adr/NNNN-<slug>.md` (how
/// the view normally cites) or a bare `ADR-NNNN` token (how prose sometimes does).
/// Both count.
///
/// There is deliberately no allowlist, exemption file, or baseline — this is a
/// pure function of the tree, matching the coverage gate's stateless stance
/// (ADR-0050). The absence of an escape hatch is the design: when an ADR is
/// named here the fix is to describe it in the view, never to exempt it.
///
/// A number cited by the view with no corresponding ADR file is a dangling link,
/// not a parity problem — only accepted ADRs are walked.
///
/// Errors when the view is unreadable: a gate that silently passes because its
/// input vanished would be retired by a `rm`.
///
/// **What this cannot see** (ADR-0085's honesty obligation). It is a substring
/// test over the whole file, so a citation counts wherever it appears —  inside a
/// fenced code block, an HTML comment, or a "superseded by ADR-NNNN" aside. It
/// therefore proves that an ADR is *mentioned*, not that the surrounding prose is
/// true, not that the mention is anywhere sensible, and not that a `superseded`
/// ADR has stopped being cited as current. Those are the replay audit's job, and
/// the reason it is not replaced by this step. The failure direction is
/// deliberate: a deliberately odd view can produce a false pass, but nothing
/// produces a false alarm, which is the right way round for a step that blocks
/// every commit.
pub fn view_parity_problems(repo: &Path) -> Result<Vec<String>> {
    let view_path = repo.join(VIEW);
    let view = std::fs::read_to_string(&view_path)
        .with_context(|| format!("reading {}", view_path.display()))?;
    let mut problems = Vec::new();
    for e in parse_adr_dir(repo)? {
        if e.status != "accepted" {
            continue;
        }
        let link = adr_link(&e.filename);
        let token = format!("ADR-{:04}", e.num);
        if !view.contains(&link) && !view.contains(&token) {
            problems.push(format!(
                "ADR {:04} ({}) is accepted but is not cited in {VIEW}",
                e.num, e.title
            ));
        }
    }
    problems.sort();
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway repo dir with `docs/adr/`, unique per (pid, tag) so parallel
    /// tests don't collide. Cleaned best-effort by the caller.
    fn scratch_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jaunder-adr-readme-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/adr")).unwrap();
        dir
    }

    /// Write `docs/ARCHITECTURE.md` in a scratch repo.
    fn write_view(repo: &Path, body: &str) {
        std::fs::write(repo.join(VIEW), body).unwrap();
    }

    #[test]
    fn view_parity_accepts_a_markdown_link_citation() {
        let repo = scratch_repo("view-link");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(
            &repo,
            "# View\n\nWe do it this way ([0001](adr/0001-a.md)).\n",
        );

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_accepts_a_bare_token_citation() {
        let repo = scratch_repo("view-token");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nThe seam is fixed by ADR-0001.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_reports_an_uncited_accepted_adr() {
        let repo = scratch_repo("view-uncited");
        std::fs::write(
            repo.join("docs/adr/0007-lonely.md"),
            "# ADR-0007: Lonely decision\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nNothing is cited here.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("0007"), "{problems:?}");
        assert!(problems[0].contains("Lonely decision"), "{problems:?}");
    }

    #[test]
    fn view_parity_ignores_a_superseded_adr() {
        let repo = scratch_repo("view-superseded");
        std::fs::write(
            repo.join("docs/adr/0002-old.md"),
            "# ADR-0002: Old decision\n\n- Status: superseded\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nNothing is cited here.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_ignores_a_citation_with_no_adr_file() {
        // The view may name a number that has no file. That is a dangling link,
        // not this step's business — the step only walks accepted ADRs.
        let repo = scratch_repo("view-dangling");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nSee ADR-0001 and also ADR-9999.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_errors_when_the_view_is_absent() {
        // A step that silently passes when its input is missing is worse than
        // useless: deleting the view would retire the gate.
        let repo = scratch_repo("view-absent");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();

        let report = view_parity_problems(&repo);
        let _ = std::fs::remove_dir_all(&repo);
        assert!(report.is_err(), "absent view must be an error");
    }
}

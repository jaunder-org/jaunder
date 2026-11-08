//! Read-only README ADR-table parsing and parity validation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};

use super::files::{AdrEntry, AdrFile, adr_link, parse_adr_files};

pub const README: &str = "docs/README.md";
pub const BEGIN: &str = "<!-- adr-table:begin -->";
pub const END: &str = "<!-- adr-table:end -->";

/// A parsed committed table row. Cells are trimmed (padding-proof).
#[derive(Debug, PartialEq, Eq)]
pub struct TableRow {
    pub num: u32,
    pub target: String,
    pub title: String,
    pub status: String,
}

/// Parse one committed table row `| [NNNN](adr/slug.md) | Title | status |`.
/// `None` for the header, the separator, and any non-row line.
fn parse_row(line: &str) -> Option<TableRow> {
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    // A table row is bounded by pipes, so the split yields empty first/last cells
    // around exactly three inner cells.
    if cells.len() != 5 || !cells[0].is_empty() || !cells[4].is_empty() {
        return None;
    }
    let (link, title, status) = (cells[1], cells[2], cells[3]);
    let link = link.strip_prefix('[')?;
    let close = link.find(']')?;
    let num: u32 = link[..close].parse().ok()?;
    let paren = link[close..].strip_prefix("](")?;
    let target = paren.strip_suffix(')')?.to_string();
    Some(TableRow {
        num,
        target,
        title: title.to_string(),
        status: status.to_string(),
    })
}

/// Parse the committed table rows out of the block text between the markers.
pub fn parse_table_block(block: &str) -> Vec<TableRow> {
    block.lines().filter_map(parse_row).collect()
}

/// The byte range strictly between the ADR-table markers: `(start, end)` where
/// `start` is just past `BEGIN` and `end` is at `END`. Errors when either marker
/// is missing or they are out of order.
fn marker_bounds(readme: &str) -> Result<(usize, usize)> {
    let begin = readme
        .find(BEGIN)
        .with_context(|| format!("{README} is missing the `{BEGIN}` marker"))?;
    let end = readme
        .find(END)
        .with_context(|| format!("{README} is missing the `{END}` marker"))?;
    anyhow::ensure!(begin < end, "{README} adr-table markers are out of order");
    Ok((begin + BEGIN.len(), end))
}

/// The block text between the markers (for reading existing titles).
fn extract_block(readme: &str) -> Result<String> {
    let (start, end) = marker_bounds(readme)?;
    Ok(readme[start..end].to_string())
}

/// The `adr-readme-parity` problems: the committed table's mechanical cells
/// (number, link target, status), row presence, and ordering must match the ADR
/// directory. Titles are not compared (they are hand-owned). Does not panic on a
/// transient duplicate number — that is `identifier-collisions`' concern.
pub fn parity_problems(entries: &[AdrEntry], existing: &[TableRow]) -> Vec<String> {
    let mut problems = Vec::new();
    let row_by_num: BTreeMap<u32, &TableRow> = existing.iter().map(|r| (r.num, r)).collect();
    let entry_nums: BTreeSet<u32> = entries.iter().map(|e| e.num).collect();

    for e in entries {
        match row_by_num.get(&e.num) {
            None => problems.push(format!("ADR {:04} has no README table row", e.num)),
            Some(r) => {
                let want = adr_link(&e.filename);
                if r.target != want {
                    problems.push(format!(
                        "ADR {:04} row link is `{}`, expected `{want}`",
                        e.num, r.target
                    ));
                }
                if r.status != e.status {
                    problems.push(format!(
                        "ADR {:04} row status is `{}`, expected `{}`",
                        e.num, r.status, e.status
                    ));
                }
            }
        }
    }
    for r in existing {
        if !entry_nums.contains(&r.num) {
            problems.push(format!(
                "README row {:04} has no matching ADR file (orphan)",
                r.num
            ));
        }
    }
    let nums: Vec<u32> = existing.iter().map(|r| r.num).collect();
    let mut ascending = nums.clone();
    ascending.sort_unstable();
    if nums != ascending {
        problems.push("README ADR rows are not in ascending number order".to_string());
    }

    problems.sort();
    problems
}

/// Read `repo`'s README + ADR directory and compute the parity problems. Errors
/// when the README is unreadable or the table markers are absent.
pub fn parity_report(repo: &Path) -> Result<Vec<String>> {
    parity_report_with(repo, || super::files::adr_files(repo))
}

pub(super) fn parity_report_with(
    repo: &Path,
    files: impl FnOnce() -> Result<Vec<AdrFile>>,
) -> Result<Vec<String>> {
    let readme_path = repo.join(README);
    let readme = std::fs::read_to_string(&readme_path)
        .with_context(|| format!("reading {}", readme_path.display()))?;
    let entries = parse_adr_files(repo, files())?;
    let existing = parse_table_block(&extract_block(&readme)?);
    Ok(parity_problems(&entries, &existing))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::super::files::{ADR_DIR, adr_files_from, format_problems};
    use super::*;

    fn entry(num: u32, file: &str, title: &str, status: &str) -> AdrEntry {
        AdrEntry {
            num,
            filename: file.into(),
            title: title.into(),
            status: status.into(),
        }
    }

    #[test]
    fn parse_row_trims_padded_cells_and_skips_non_rows() {
        let r = parse_row("| [0007](adr/0007-auth-mechanisms.md)   | Dual-Path Auth | accepted |")
            .expect("a row");
        assert_eq!(r.num, 7);
        assert_eq!(r.target, "adr/0007-auth-mechanisms.md");
        assert_eq!(r.title, "Dual-Path Auth");
        assert_eq!(r.status, "accepted");
        assert!(parse_row("| # | Title | Status |").is_none());
        assert!(parse_row("| --- | --- | --- |").is_none());
        assert!(parse_row("plain text").is_none());
    }

    #[test]
    fn parity_problems_flags_mechanical_drift_but_ignores_titles() {
        let entries = vec![
            entry(1, "0001-a.md", "H1", "accepted"),
            entry(2, "0002-b.md", "H2", "superseded"),
        ];
        // Row 1: title differs (OK — not compared) but everything mechanical agrees.
        // Row 2: status is stale. Plus an orphan row 9.
        let existing = vec![
            TableRow {
                num: 1,
                target: "adr/0001-a.md".into(),
                title: "Totally Different".into(),
                status: "accepted".into(),
            },
            TableRow {
                num: 2,
                target: "adr/0002-b.md".into(),
                title: "H2".into(),
                status: "accepted".into(),
            },
            TableRow {
                num: 9,
                target: "adr/0009-x.md".into(),
                title: "Ghost".into(),
                status: "accepted".into(),
            },
        ];
        let problems = parity_problems(&entries, &existing);
        assert!(
            problems.iter().any(|p| p.contains("ADR 0002 row status")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("orphan")),
            "{problems:?}"
        );
        assert!(
            !problems.iter().any(|p| p.contains("0001")),
            "title-only diff must not flag: {problems:?}"
        );
    }

    #[test]
    fn parity_problems_flags_missing_row_and_bad_ordering() {
        let entries = vec![
            entry(1, "0001-a.md", "H1", "accepted"),
            entry(2, "0002-b.md", "H2", "accepted"),
        ];
        // Row for ADR 2 is missing; the two present rows are out of order.
        let existing = vec![
            TableRow {
                num: 3,
                target: "adr/0003-c.md".into(),
                title: "T3".into(),
                status: "accepted".into(),
            },
            TableRow {
                num: 1,
                target: "adr/0001-a.md".into(),
                title: "T1".into(),
                status: "accepted".into(),
            },
        ];
        let problems = parity_problems(&entries, &existing);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("ADR 0002 has no README table row")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("ascending")),
            "{problems:?}"
        );
    }

    #[test]
    fn parity_problems_does_not_panic_on_duplicate_number() {
        // The always-0000 sentinel: two entries share num 0. Must not panic.
        let entries = vec![
            entry(0, "0000-doc.md", "Doc", "accepted"),
            entry(0, "0000-new.md", "New", "accepted"),
        ];
        let existing = vec![TableRow {
            num: 0,
            target: "adr/0000-doc.md".into(),
            title: "Doc".into(),
            status: "accepted".into(),
        }];
        let _ = parity_problems(&entries, &existing);
    }

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

    #[test]
    fn fail_closed_population_unreadable_adr_readme_file_type() {
        struct Fake {
            path: PathBuf,
            name: OsString,
        }
        let repo = scratch_repo("adr-file-type-owner");
        std::fs::create_dir_all(repo.join(ADR_DIR)).unwrap();
        std::fs::write(repo.join(README), format!("# ADRs\n{BEGIN}\n{END}\n")).unwrap();
        let path = repo.join("docs/adr/0001-unreadable.md");
        let report = parity_report_with(&repo, || {
            adr_files_from(
                [Ok(Fake {
                    path: path.clone(),
                    name: OsString::from("0001-unreadable.md"),
                })],
                |entry| entry.path.clone(),
                |entry| entry.name.clone(),
                |_| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "injected",
                    ))
                },
            )
        });
        let error = match report {
            Ok(_) => panic!("injected file-type failure must fail the parity population"),
            Err(error) => error,
        };
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
        let step = crate::steps::adr_check::parity_step(Err(error));
        assert!(!step.ok);
        let detail = step.detail.unwrap();
        assert!(detail.contains(&path.display().to_string()), "{detail}");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn parity_report_reads_readme_and_errors_without_markers() {
        let repo = scratch_repo("parity-report");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();

        // Markers present, mechanical cells agree (title free to differ): clean.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | Curated | accepted |\n\n{END}\n"
            ),
        )
        .unwrap();
        assert!(parity_report(&repo).unwrap().is_empty());

        // A stale status cell is reported.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | Curated | proposed |\n\n{END}\n"
            ),
        )
        .unwrap();
        let problems = parity_report(&repo).unwrap();
        assert!(
            problems.iter().any(|p| p.contains("ADR 0001 row status")),
            "{problems:?}"
        );

        // No markers at all: an error, not a silent empty report.
        std::fs::write(repo.join("docs/README.md"), "# Docs\n\nno table here\n").unwrap();
        assert!(parity_report(&repo).is_err());

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn gates_ignore_docs_adr_template_md() {
        // `docs/adr/template.md` (the copyable ADR skeleton, #207) has no leading
        // number, so both gate entry points must skip it: it is neither an
        // `adr-format` subject nor an `adr-readme-parity` row. Guards against a
        // refactor that starts checking `docs/adr/*.md` regardless of number.
        //
        // Teeth: rename the fixture to `0099-template.md` and this fails — the
        // `# ADR-0000:` heading mismatches `0099` (adr-format) and no README row
        // exists for 0099 (parity). See the plan's inversion check.
        let repo = scratch_repo("template-ignored");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        std::fs::write(
            repo.join("docs/adr/template.md"),
            "# ADR-0000: Title of the decision\n\n- Status: proposed\n",
        )
        .unwrap();
        // README carries only the real ADR's row — none for the template.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | First | accepted |\n\n{END}\n"
            ),
        )
        .unwrap();

        let fmt = format_problems(&repo);
        let parity = parity_report(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            fmt.is_empty(),
            "template.md must not be an adr-format subject: {fmt:?}"
        );
        assert!(
            parity.is_empty(),
            "template.md must not be a parity row: {parity:?}"
        );
    }

    #[test]
    fn gates_ignore_docs_adr_drafts_subdir() {
        // Feature PRs track numberless files under `docs/adr/drafts/`; the
        // serialized promoter numbers them after feature merge. The subdirectory
        // is excluded twice over by the shared enumeration rule (a non-recursive
        // `read_dir` skips the subdir entry, which is not a file; and there is no
        // leading number), so a draft — even one that violates every ADR format
        // rule (no `# ADR-NNNN:` heading, no status line) — must never trip
        // `adr-format` or `adr-readme-parity`.
        //
        // Teeth: move the fixture up to `docs/adr/0099-some-decision.md` and this
        // fails — the `# ADR-DRAFT:` heading mismatches 0099 (adr-format) and no
        // README row exists for 0099 (parity).
        let repo = scratch_repo("drafts-ignored");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        std::fs::create_dir_all(repo.join("docs/adr/drafts")).unwrap();
        std::fs::write(
            repo.join("docs/adr/drafts/some-decision.md"),
            "# ADR-DRAFT: Some Decision\n\nNo status line, no number — still invisible.\n",
        )
        .unwrap();
        // README carries only the real ADR's row — none for the draft.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | First | accepted |\n\n{END}\n"
            ),
        )
        .unwrap();

        let fmt = format_problems(&repo);
        let parity = parity_report(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            fmt.is_empty(),
            "a drafts/ entry must not be an adr-format subject: {fmt:?}"
        );
        assert!(
            parity.is_empty(),
            "a drafts/ entry must not be a parity row: {parity:?}"
        );
    }
}

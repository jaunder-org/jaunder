//! Generate the ADR index table in `docs/README.md` as a projection of
//! `docs/adr/`. Only the mechanical cells — number, link target, status — are
//! generated; the title cell is hand-curated and preserved (seeded from the ADR
//! heading when a row is first created). The table lives between HTML-comment
//! markers so only that block is ever rewritten.
//!
//! This core is shared by `adr sync-readme` (the writer, here), `adr renumber`
//! (which regenerates the table after a collision bump), and the read-only
//! parity gate. No behavior lives in more than one place.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids;
use crate::result::StepResult;

pub const README: &str = "docs/README.md";
pub const ADR_DIR: &str = "docs/adr";
pub const BEGIN: &str = "<!-- adr-table:begin -->";
pub const END: &str = "<!-- adr-table:end -->";

/// The recognized ADR status tokens (the canonical status cell is exactly one).
pub const STATUS_VOCAB: [&str; 5] = [
    "proposed",
    "accepted",
    "superseded",
    "deprecated",
    "rejected",
];

/// An ADR file projected to its table-relevant fields. `title` is the heading
/// text with the `ADR-NNNN:` / `NNNN.` prefix stripped (used only to seed a new
/// row); `status` is the single status token.
pub struct AdrEntry {
    pub num: u32,
    pub filename: String,
    pub title: String,
    pub status: String,
}

/// A parsed committed table row. Cells are trimmed (padding-proof).
#[derive(Debug, PartialEq, Eq)]
pub struct TableRow {
    pub num: u32,
    pub target: String,
    pub title: String,
    pub status: String,
}

/// A directory entry that qualifies as an ADR: a regular `*.md` file whose name
/// carries a leading number. Content is intentionally not read here so each
/// caller keeps its own IO-error policy.
struct AdrFile {
    num: u32,
    filename: String,
    path: PathBuf,
}

/// The qualifying ADR files under `repo/docs/adr`, unsorted — the single home of
/// the "what counts as an ADR file" rule (`is_file` → `.md` → leading number).
/// The `read_dir` error is returned unwrapped so callers phrase their own context
/// (`parse_adr_dir` wants "reading <dir>", `format_problems` wants
/// "cannot read <dir>").
fn adr_files(repo: &Path) -> Result<Vec<AdrFile>> {
    let mut files = Vec::new();
    for ent in std::fs::read_dir(repo.join(ADR_DIR))? {
        let ent = ent?;
        if !ent.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let filename = ent.file_name().to_string_lossy().into_owned();
        if !filename.ends_with(".md") {
            continue;
        }
        let Some(num) = ids::leading_number(&filename) else {
            continue;
        };
        files.push(AdrFile {
            num,
            filename,
            path: ent.path(),
        });
    }
    Ok(files)
}

/// The title text of a `# ADR-NNNN: Title` (or legacy `# NNNN. Title`) heading,
/// prefix stripped. Falls back to the whole heading when it matches neither form.
fn heading_title(content: &str) -> String {
    let line = content.lines().find(|l| l.starts_with("# ")).unwrap_or("");
    let after = line.trim_start_matches("# ").trim();
    for (prefix, sep) in [("ADR-", ": "), ("", ". ")] {
        if let Some((lhs, title)) = after.split_once(sep) {
            if lhs.starts_with(prefix)
                && !lhs.is_empty()
                && lhs[prefix.len()..].chars().all(|c| c.is_ascii_digit())
            {
                return title.trim().to_string();
            }
        }
    }
    after.to_string()
}

/// The single status token from a `- Status: <token>` line (leniently, also a
/// bare `Status:` line), or `""` when none is found.
fn status_token(content: &str) -> String {
    let line = content
        .lines()
        .map(str::trim_start)
        .find(|l| l.starts_with("- Status:") || l.starts_with("Status:"))
        .unwrap_or("");
    line.trim_start_matches("- Status:")
        .trim_start_matches("Status:")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string()
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

/// Parse the ADR files under `repo/docs/adr`, sorted ascending by number.
pub fn parse_adr_dir(repo: &Path) -> Result<Vec<AdrEntry>> {
    let dir = repo.join(ADR_DIR);
    let mut entries = Vec::new();
    for f in adr_files(repo).with_context(|| format!("reading {}", dir.display()))? {
        let content = std::fs::read_to_string(&f.path)
            .with_context(|| format!("reading {}", f.path.display()))?;
        entries.push(AdrEntry {
            num: f.num,
            filename: f.filename,
            title: heading_title(&content),
            status: status_token(&content),
        });
    }
    entries.sort_by_key(|e| e.num);
    Ok(entries)
}

/// Parse the committed table rows out of the block text between the markers.
pub fn parse_table_block(block: &str) -> Vec<TableRow> {
    block.lines().filter_map(parse_row).collect()
}

/// The generated table block: header + separator + one row per ADR entry
/// (ascending), reusing an existing row's title when present, else seeding the
/// title from the ADR heading. Single-space padded — prettier owns alignment.
pub fn render_block(entries: &[AdrEntry], existing: &[TableRow]) -> String {
    let mut out = String::from("| #   | Title | Status |\n| --- | ----- | ------ |\n");
    for r in resolved_rows(entries, existing) {
        out.push_str(&format!(
            "| [{:04}]({}) | {} | {} |\n",
            r.num, r.target, r.title, r.status
        ));
    }
    out.trim_end().to_string()
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

/// Replace the text strictly between the markers with `new_block`. Errors when a
/// marker is missing or out of order.
pub fn splice_block(readme: &str, new_block: &str) -> Result<String> {
    let (after_begin, end) = marker_bounds(readme)?;
    Ok(format!(
        "{}\n\n{}\n\n{}",
        &readme[..after_begin],
        new_block,
        &readme[end..]
    ))
}

/// The block text between the markers (for reading existing titles).
fn extract_block(readme: &str) -> Result<String> {
    let (start, end) = marker_bounds(readme)?;
    Ok(readme[start..end].to_string())
}

/// The desired table rows, ascending by number, applying the title-preservation
/// rule once: reuse an existing row's title when a row with that number exists,
/// else seed it from the ADR heading. The single source of that rule — both the
/// renderer ([`render_block`]) and the idempotence check ([`sync_readme_at`])
/// consume it, so they can never disagree.
fn resolved_rows(entries: &[AdrEntry], existing: &[TableRow]) -> Vec<TableRow> {
    let title_by_num: BTreeMap<u32, &str> =
        existing.iter().map(|r| (r.num, r.title.as_str())).collect();
    let mut sorted: Vec<&AdrEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.num);
    sorted
        .into_iter()
        .map(|e| {
            let title = title_by_num
                .get(&e.num)
                .copied()
                .unwrap_or(e.title.as_str())
                .to_string();
            TableRow {
                num: e.num,
                target: format!("adr/{}", e.filename),
                title,
                status: e.status.clone(),
            }
        })
        .collect()
}

/// Regenerate the ADR table in `repo/docs/README.md` from `repo/docs/adr`.
/// A no-op (no write) when the table already matches semantically, so it is
/// idempotent regardless of prettier's column padding. Returns a human summary.
pub fn sync_readme_at(repo: &Path) -> Result<String> {
    let readme_path = repo.join(README);
    let readme = std::fs::read_to_string(&readme_path)
        .with_context(|| format!("reading {}", readme_path.display()))?;
    let entries = parse_adr_dir(repo)?;
    let existing = parse_table_block(&extract_block(&readme)?);

    let desired = resolved_rows(&entries, &existing);
    if desired == existing {
        return Ok(format!("{} rows, already in sync", entries.len()));
    }

    let updated = splice_block(&readme, &render_block(&entries, &existing))?;
    std::fs::write(&readme_path, &updated)
        .with_context(|| format!("writing {}", readme_path.display()))?;

    let existing_nums: BTreeSet<u32> = existing.iter().map(|r| r.num).collect();
    let entry_nums: BTreeSet<u32> = entries.iter().map(|e| e.num).collect();
    let added = entry_nums.difference(&existing_nums).count();
    let removed = existing_nums.difference(&entry_nums).count();
    Ok(format!(
        "{} rows ({added} added, {removed} removed)",
        entries.len()
    ))
}

/// Whether `repo`'s README carries the ADR-table markers. `Ok(false)` when the
/// README is absent (a scratch/test repo may omit it entirely) so the caller can
/// skip table sync; a genuine read error still propagates rather than being
/// mistaken for "no markers".
pub fn readme_has_markers(repo: &Path) -> Result<bool> {
    let readme_path = repo.join(README);
    match std::fs::read_to_string(&readme_path) {
        Ok(s) => Ok(s.contains(BEGIN) && s.contains(END)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("reading {}", readme_path.display())),
    }
}

/// Entry point for `cargo xtask adr sync-readme`.
pub fn sync_readme() -> StepResult {
    match sync_readme_at(Path::new(".")) {
        Ok(summary) => StepResult::ok("adr-sync-readme").detail(summary),
        Err(e) => StepResult::fail("adr-sync-readme").detail(format!("{e:#}")),
    }
}

/// The `adr-format` problems for one ADR file: the line-1 heading must be
/// `# ADR-NNNN: <nonempty>` with `NNNN` matching the filename number, and a
/// `- Status: <token>` line must exist with a single token from [`STATUS_VOCAB`]
/// and nothing trailing. `filename`/`num` come from the directory entry.
fn file_format_problems(filename: &str, num: u32, content: &str) -> Vec<String> {
    let mut problems = Vec::new();

    let line1 = content.lines().next().unwrap_or("");
    let prefix = format!("# ADR-{:04}: ", num);
    match line1.strip_prefix(&prefix) {
        Some(title) if !title.trim().is_empty() => {}
        Some(_) => problems.push(format!("{filename}: heading has an empty title")),
        None => problems.push(format!(
            "{filename}: heading must be `# ADR-{:04}: <title>` (found `{line1}`)",
            num
        )),
    }

    match content.lines().find(|l| l.starts_with("- Status:")) {
        None => problems.push(format!("{filename}: missing a `- Status: <token>` line")),
        Some(l) => {
            let rest = l.strip_prefix("- Status:").unwrap_or("").trim();
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.len() != 1 {
                problems.push(format!(
                    "{filename}: `- Status:` must be a single token with nothing trailing (found `{rest}`)"
                ));
            } else if !STATUS_VOCAB.contains(&tokens[0]) {
                problems.push(format!(
                    "{filename}: status `{}` is not one of {STATUS_VOCAB:?}",
                    tokens[0]
                ));
            }
        }
    }
    problems
}

/// Every ADR file's `adr-format` problems, sorted for stable output. A directory
/// read error is surfaced as a single problem rather than a panic.
pub fn format_problems(repo: &Path) -> Vec<String> {
    let files = match adr_files(repo) {
        Ok(f) => f,
        Err(e) => return vec![format!("cannot read {}: {e}", repo.join(ADR_DIR).display())],
    };
    let mut problems = Vec::new();
    for f in files {
        match std::fs::read_to_string(&f.path) {
            Ok(content) => problems.extend(file_format_problems(&f.filename, f.num, &content)),
            Err(e) => problems.push(format!("{}: cannot read ({e})", f.filename)),
        }
    }
    problems.sort();
    problems
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
                let want = format!("adr/{}", e.filename);
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
    let readme_path = repo.join(README);
    let readme = std::fs::read_to_string(&readme_path)
        .with_context(|| format!("reading {}", readme_path.display()))?;
    let entries = parse_adr_dir(repo)?;
    let existing = parse_table_block(&extract_block(&readme)?);
    Ok(parity_problems(&entries, &existing))
}

#[cfg(test)]
mod tests {
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
    fn heading_title_strips_canonical_and_legacy_prefixes() {
        assert_eq!(
            heading_title("# ADR-0021: SQLite discipline: avoid deferred txns\n"),
            "SQLite discipline: avoid deferred txns"
        );
        assert_eq!(
            heading_title("# 0030. Coverage re-anchor by text identity\n"),
            "Coverage re-anchor by text identity"
        );
    }

    #[test]
    fn status_token_reads_list_and_bare_forms() {
        assert_eq!(
            status_token("# T\n\n- Status: accepted\n- Note: x\n"),
            "accepted"
        );
        assert_eq!(status_token("# T\n\nStatus: superseded\n"), "superseded");
        assert_eq!(status_token("# T\n\nno status here\n"), "");
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
    fn render_block_preserves_existing_title_and_seeds_new_from_heading() {
        let entries = vec![
            entry(1, "0001-a.md", "Heading One", "accepted"),
            entry(2, "0002-b.md", "Heading Two", "accepted"),
        ];
        let existing = vec![TableRow {
            num: 1,
            target: "adr/0001-a.md".into(),
            title: "Curated One".into(),
            status: "accepted".into(),
        }];
        let block = render_block(&entries, &existing);
        // Existing row keeps its curated title; the new row seeds from the heading.
        assert!(block.contains("| [0001](adr/0001-a.md) | Curated One | accepted |"));
        assert!(block.contains("| [0002](adr/0002-b.md) | Heading Two | accepted |"));
    }

    #[test]
    fn render_block_drops_orphans_and_sorts_ascending() {
        let entries = vec![
            entry(3, "0003-c.md", "Three", "accepted"),
            entry(1, "0001-a.md", "One", "accepted"),
        ];
        // An existing row for a now-deleted ADR 2 must not survive.
        let existing = vec![TableRow {
            num: 2,
            target: "adr/0002-b.md".into(),
            title: "Two".into(),
            status: "accepted".into(),
        }];
        let block = render_block(&entries, &existing);
        let one = block.find("0001-a.md").unwrap();
        let three = block.find("0003-c.md").unwrap();
        assert!(one < three, "ascending order");
        assert!(!block.contains("0002-b.md"), "orphan dropped");
    }

    #[test]
    fn splice_block_replaces_only_between_markers() {
        let readme = format!("intro\n\n{BEGIN}\n\nOLD TABLE\n\n{END}\n\noutro\n");
        let out = splice_block(&readme, "NEW TABLE").unwrap();
        assert!(out.contains("intro\n"));
        assert!(out.contains("outro\n"));
        assert!(out.contains(&format!("{BEGIN}\n\nNEW TABLE\n\n{END}")));
        assert!(!out.contains("OLD TABLE"));
    }

    #[test]
    fn splice_block_errors_on_missing_marker() {
        let err = splice_block("no markers here", "x").unwrap_err();
        assert!(format!("{err:#}").contains("marker"));
    }

    #[test]
    fn desired_matches_current_when_in_sync() {
        let entries = vec![entry(1, "0001-a.md", "Heading", "accepted")];
        let existing = vec![TableRow {
            num: 1,
            target: "adr/0001-a.md".into(),
            title: "Curated".into(),
            status: "accepted".into(),
        }];
        // The preserved title is the curated one, so desired == current: a no-op.
        assert_eq!(resolved_rows(&entries, &existing), existing);
    }

    #[test]
    fn file_format_problems_flags_each_violation() {
        // Clean file: no problems.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\n- Status: accepted\n")
                .is_empty()
        );
        // Legacy heading form.
        assert!(
            file_format_problems("0007-a.md", 7, "# 0007. Auth\n\n- Status: accepted\n")
                .iter()
                .any(|p| p.contains("heading must be"))
        );
        // Filename/heading number mismatch.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0008: Auth\n\n- Status: accepted\n")
                .iter()
                .any(|p| p.contains("heading must be"))
        );
        // Missing status.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\nbody\n")
                .iter()
                .any(|p| p.contains("missing a `- Status:"))
        );
        // Trailing prose after the token.
        assert!(file_format_problems(
            "0007-a.md",
            7,
            "# ADR-0007: Auth\n\n- Status: accepted (superseded)\n"
        )
        .iter()
        .any(|p| p.contains("single token")));
        // Out-of-vocabulary token.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\n- Status: accpeted\n")
                .iter()
                .any(|p| p.contains("not one of"))
        );
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
    fn parse_adr_dir_reads_sorts_and_skips_non_adrs() {
        let repo = scratch_repo("parse-dir");
        let adr = repo.join("docs/adr");
        std::fs::write(
            adr.join("0002-b.md"),
            "# ADR-0002: Second\n\n- Status: superseded\n",
        )
        .unwrap();
        std::fs::write(
            adr.join("0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        // Skipped: not markdown, and markdown without a leading number.
        std::fs::write(adr.join("0003-c.txt"), "ignore me").unwrap();
        std::fs::write(adr.join("template.md"), "# ADR-template\n").unwrap();

        let entries = parse_adr_dir(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        let projected: Vec<_> = entries
            .iter()
            .map(|e| {
                (
                    e.num,
                    e.filename.as_str(),
                    e.title.as_str(),
                    e.status.as_str(),
                )
            })
            .collect();
        assert_eq!(
            projected,
            vec![
                (1, "0001-a.md", "First", "accepted"),
                (2, "0002-b.md", "Second", "superseded"),
            ]
        );
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
    fn readme_has_markers_distinguishes_absent_present_and_missing() {
        let repo = scratch_repo("has-markers");
        // No README file: absent, reported as false (not an error).
        assert!(!readme_has_markers(&repo).unwrap());
        // Present markers.
        std::fs::write(
            repo.join("docs/README.md"),
            format!("# Docs\n\n{BEGIN}\n\n{END}\n"),
        )
        .unwrap();
        assert!(readme_has_markers(&repo).unwrap());
        // README exists but carries no markers.
        std::fs::write(repo.join("docs/README.md"), "# Docs\n").unwrap();
        assert!(!readme_has_markers(&repo).unwrap());
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
}
